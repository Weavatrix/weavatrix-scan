use crate::cache::{ScanCache, ScanCacheEntry};
use crate::config::{CacheValidationPolicy, EvidenceMode, ScanOptions};
use crate::content_visit::{ContentVisitControl, ContentVisitEvent, ContentVisitMode};
use crate::error::{Error, Result};
use crate::file_version::reusable;
use crate::report::{
    CompactContentEvidence, CompactScannedFile, ScanCacheStats, ScanTermination, ScanWarning,
    ScannedFile, SkipKind, SkippedEntry,
};
use crate::walker::ErrorPolicy;
use inspect::{Inspection, VisitedStatus};
use std::collections::HashMap;
use std::io;
use std::path::Path;
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::Instant;

mod inspect;

#[derive(Clone, Copy)]
pub(crate) struct ContentWorkerContext<'a> {
    pub(crate) root: &'a Path,
    pub(crate) root_index: usize,
    pub(crate) worker_index: usize,
    pub(crate) mode: ContentVisitMode,
}

pub(crate) struct InspectedFiles {
    pub(crate) files: Vec<ScannedFile>,
    pub(crate) skipped: Vec<SkippedEntry>,
    pub(crate) warnings: Vec<ScanWarning>,
    pub(crate) termination: Option<ScanTermination>,
    pub(crate) cache: ScanCacheStats,
}

pub(crate) struct VisitedFiles {
    pub(crate) files: Vec<(u64, CompactScannedFile)>,
    pub(crate) evidence: InspectedFiles,
    pub(crate) opened: u64,
    pub(crate) chunks: u64,
    pub(crate) bytes_read: u64,
    pub(crate) bytes_emitted: u64,
    pub(crate) consumer_skipped: u64,
    pub(crate) completed: u64,
    pub(crate) visitor_quit: bool,
}

impl VisitedFiles {
    pub(crate) fn empty(capacity: usize) -> Self {
        Self {
            files: Vec::with_capacity(capacity),
            evidence: InspectedFiles {
                files: Vec::new(),
                skipped: Vec::new(),
                warnings: Vec::new(),
                termination: None,
                cache: ScanCacheStats::default(),
            },
            opened: 0,
            chunks: 0,
            bytes_read: 0,
            bytes_emitted: 0,
            consumer_skipped: 0,
            completed: 0,
            visitor_quit: false,
        }
    }

    pub(crate) fn merge(&mut self, other: Self) {
        self.files.extend(other.files);
        self.evidence.skipped.extend(other.evidence.skipped);
        self.evidence.warnings.extend(other.evidence.warnings);
        self.evidence.termination = self.evidence.termination.or(other.evidence.termination);
        self.evidence.cache.reused_hashes = self
            .evidence
            .cache
            .reused_hashes
            .saturating_add(other.evidence.cache.reused_hashes);
        self.evidence.cache.content_reads = self
            .evidence
            .cache
            .content_reads
            .saturating_add(other.evidence.cache.content_reads);
        self.evidence.cache.fingerprint_reads = self
            .evidence
            .cache
            .fingerprint_reads
            .saturating_add(other.evidence.cache.fingerprint_reads);
        self.opened = self.opened.saturating_add(other.opened);
        self.chunks = self.chunks.saturating_add(other.chunks);
        self.bytes_read = self.bytes_read.saturating_add(other.bytes_read);
        self.bytes_emitted = self.bytes_emitted.saturating_add(other.bytes_emitted);
        self.consumer_skipped = self.consumer_skipped.saturating_add(other.consumer_skipped);
        self.completed = self.completed.saturating_add(other.completed);
        self.visitor_quit |= other.visitor_quit;
    }
}

pub(crate) fn inspect_files(
    files: Vec<ScannedFile>,
    options: &ScanOptions,
    started: Instant,
    previous: Option<&ScanCache>,
) -> Result<InspectedFiles> {
    if !options.hash_file_contents && !options.detect_binary_files {
        return Ok(InspectedFiles {
            files,
            skipped: Vec::new(),
            warnings: Vec::new(),
            termination: None,
            cache: ScanCacheStats::default(),
        });
    }
    let cache = previous.map_or_else(HashMap::new, |cache| {
        cache
            .entries
            .iter()
            .map(|entry| (entry.relative.as_str(), entry))
            .collect()
    });
    let stop = InspectionStop::default();
    let workers = options.worker_count(files.len());
    if workers <= 1 {
        return inspect_chunk(files, options, started, &stop, &cache);
    }
    let chunk_size = files.len().div_ceil(workers);
    let mut iterator = files.into_iter();
    let mut chunks = Vec::with_capacity(workers);
    loop {
        let chunk = iterator.by_ref().take(chunk_size).collect::<Vec<_>>();
        if chunk.is_empty() {
            break;
        }
        chunks.push(chunk);
    }

    std::thread::scope(|scope| {
        let handles = chunks
            .into_iter()
            .map(|chunk| scope.spawn(|| inspect_chunk(chunk, options, started, &stop, &cache)))
            .collect::<Vec<_>>();
        let mut inspected = InspectedFiles {
            files: Vec::new(),
            skipped: Vec::new(),
            warnings: Vec::new(),
            termination: None,
            cache: ScanCacheStats::default(),
        };
        for handle in handles {
            let chunk = handle.join().expect("content inspection worker panicked")?;
            inspected.files.extend(chunk.files);
            inspected.skipped.extend(chunk.skipped);
            inspected.warnings.extend(chunk.warnings);
            inspected.termination = inspected.termination.or(chunk.termination);
            inspected.cache.reused_hashes = inspected
                .cache
                .reused_hashes
                .saturating_add(chunk.cache.reused_hashes);
            inspected.cache.content_reads = inspected
                .cache
                .content_reads
                .saturating_add(chunk.cache.content_reads);
            inspected.cache.fingerprint_reads = inspected
                .cache
                .fingerprint_reads
                .saturating_add(chunk.cache.fingerprint_reads);
        }
        Ok(inspected)
    })
}

fn inspect_chunk(
    files: Vec<ScannedFile>,
    options: &ScanOptions,
    started: Instant,
    stop: &InspectionStop,
    cache: &HashMap<&str, &ScanCacheEntry>,
) -> Result<InspectedFiles> {
    let mut inspected = InspectedFiles {
        files: Vec::with_capacity(files.len()),
        skipped: Vec::new(),
        warnings: Vec::new(),
        termination: None,
        cache: ScanCacheStats::default(),
    };
    for mut file in files {
        if let Some(reason) = stop.reason(options, started) {
            record_limit_skip(&mut inspected, file.relative, reason, options);
            inspected.termination = Some(reason);
            continue;
        }
        let cached = cache.get(file.relative.as_str()).copied();
        if reusable_candidate(&file, options, cached) {
            let cached = cached.expect("reusable candidate has cache evidence");
            if options.cache_validation == CacheValidationPolicy::Fast {
                apply_cached(&mut file, cached);
                inspected.cache.reused_hashes = inspected.cache.reused_hashes.saturating_add(1);
                inspected.files.push(file);
                continue;
            }
            inspected.cache.content_reads = inspected.cache.content_reads.saturating_add(1);
            inspected.cache.fingerprint_reads = inspected.cache.fingerprint_reads.saturating_add(1);
            match inspect::validate_cached(&mut file, &cached.content_fingerprint) {
                Ok(inspect::CachedValidation::Match) => {
                    apply_cached(&mut file, cached);
                    inspected.cache.reused_hashes = inspected.cache.reused_hashes.saturating_add(1);
                    inspected.files.push(file);
                    continue;
                }
                Ok(inspect::CachedValidation::Changed) => {}
                Ok(inspect::CachedValidation::Concurrent) => {
                    record_concurrent_modification(&mut inspected, file.relative, options)?;
                    continue;
                }
                Err(source) => {
                    record_io_error(
                        &mut inspected,
                        &file,
                        "validate cached content",
                        source,
                        options,
                    )?;
                    continue;
                }
            }
        }
        inspected.cache.content_reads = inspected.cache.content_reads.saturating_add(1);
        let error_file = file.clone();
        match inspect::inspect(file, options) {
            Ok(Inspection::Selected(file)) => inspected.files.push(file),
            Ok(Inspection::Binary(relative)) => {
                record_binary_skip(&mut inspected, relative, options);
            }
            Ok(Inspection::Concurrent(relative)) => {
                record_concurrent_modification(&mut inspected, relative, options)?;
            }
            Err(source) => {
                record_io_error(
                    &mut inspected,
                    &error_file,
                    "inspect file content",
                    source,
                    options,
                )?;
            }
        }
    }
    Ok(inspected)
}

#[allow(clippy::too_many_lines)]
pub(crate) fn visit_files<V>(
    files: Vec<(u64, CompactScannedFile)>,
    options: &ScanOptions,
    started: Instant,
    context: ContentWorkerContext<'_>,
    buffer: &mut [u8],
    visitor: &mut V,
) -> Result<VisitedFiles>
where
    V: for<'event> FnMut(ContentVisitEvent<'event>) -> ContentVisitControl,
{
    let mut visited = VisitedFiles::empty(files.len());
    let stop = InspectionStop::default();
    let mut iterator = files.into_iter();
    while let Some((sequence, mut compact)) = iterator.next() {
        if let Some(reason) = stop.reason(options, started) {
            record_limit_skip(
                &mut visited.evidence,
                compact.relative.into(),
                reason,
                options,
            );
            if options.evidence == EvidenceMode::Complete {
                visited
                    .evidence
                    .skipped
                    .extend(iterator.map(|(_, file)| SkippedEntry {
                        relative: file.relative.into(),
                        kind: SkipKind::ScanLimit,
                        detail: Some(format!("{reason:?}")),
                    }));
            }
            visited.evidence.termination = Some(reason);
            break;
        }
        let discovery = compact
            .content
            .take()
            .expect("content visit retains file-version evidence");
        let relative = compact.relative.into_string();
        let mut scanned = ScannedFile {
            absolute: context.root.join(&relative),
            relative,
            bytes: compact.bytes,
            content_hash: None,
            content_fingerprint: None,
            version: discovery.version,
            binary_checked: false,
        };
        visited.evidence.cache.content_reads =
            visited.evidence.cache.content_reads.saturating_add(1);
        match inspect::inspect_with_visitor(
            &mut scanned,
            options,
            context,
            sequence,
            buffer,
            visitor,
        ) {
            Ok(result) => {
                let cancelled_without_result = result.status.is_none()
                    && options
                        .cancellation
                        .as_ref()
                        .is_some_and(crate::CancellationToken::is_cancelled);
                visited.opened = visited.opened.saturating_add(result.opened);
                visited.chunks = visited.chunks.saturating_add(result.chunks);
                visited.bytes_read = visited.bytes_read.saturating_add(result.bytes_read);
                visited.bytes_emitted = visited.bytes_emitted.saturating_add(result.bytes_emitted);
                if result.consumer_skipped {
                    visited.consumer_skipped = visited.consumer_skipped.saturating_add(1);
                }
                match result.status {
                    Some(VisitedStatus::Selected) => {
                        visited.completed = visited.completed.saturating_add(1);
                        if context.mode == ContentVisitMode::Revision {
                            visited.files.push((
                                sequence,
                                CompactScannedFile {
                                    relative: scanned.relative.into_boxed_str(),
                                    bytes: scanned.bytes,
                                    content: (options.hash_file_contents
                                        || options.detect_binary_files)
                                        .then(|| {
                                            Box::new(CompactContentEvidence {
                                                content_hash: scanned
                                                    .content_hash
                                                    .map(String::into_boxed_str),
                                                content_fingerprint: scanned
                                                    .content_fingerprint
                                                    .map(String::into_boxed_str),
                                                version: scanned.version,
                                                binary_checked: scanned.binary_checked,
                                            })
                                        }),
                                },
                            ));
                        }
                    }
                    Some(VisitedStatus::Binary) => {
                        record_binary_skip(&mut visited.evidence, scanned.relative, options);
                    }
                    Some(VisitedStatus::Concurrent) => {
                        record_concurrent_modification(
                            &mut visited.evidence,
                            scanned.relative,
                            options,
                        )?;
                    }
                    None => {}
                }
                if result.visitor_quit {
                    visited.visitor_quit = true;
                    visited.evidence.termination = Some(ScanTermination::Cancelled);
                    break;
                }
                if cancelled_without_result {
                    visited.evidence.termination = Some(ScanTermination::Cancelled);
                    break;
                }
            }
            Err(source) => {
                record_io_error(
                    &mut visited.evidence,
                    &scanned,
                    "visit file content",
                    source,
                    options,
                )?;
            }
        }
    }
    Ok(visited)
}

fn reusable_candidate(
    current: &ScannedFile,
    options: &ScanOptions,
    previous: Option<&ScanCacheEntry>,
) -> bool {
    let Some(previous) = previous else {
        return false;
    };
    if !previous.content_hash.starts_with("sha256:") {
        return false;
    }
    if current.bytes != previous.bytes
        || !reusable(&previous.version, &current.version)
        || (options.detect_binary_files && !previous.binary_checked)
    {
        return false;
    }
    true
}

fn apply_cached(current: &mut ScannedFile, previous: &ScanCacheEntry) {
    current.content_hash = Some(previous.content_hash.clone());
    current.content_fingerprint = Some(previous.content_fingerprint.clone());
    current.binary_checked = previous.binary_checked;
}

#[derive(Default)]
struct InspectionStop(AtomicU8);

impl InspectionStop {
    fn reason(&self, options: &ScanOptions, started: Instant) -> Option<ScanTermination> {
        if let Some(reason) = termination_from_code(self.0.load(Ordering::Acquire)) {
            return Some(reason);
        }
        let reason = if options
            .cancellation
            .as_ref()
            .is_some_and(crate::CancellationToken::is_cancelled)
        {
            Some(ScanTermination::Cancelled)
        } else if options
            .limits
            .timeout
            .is_some_and(|timeout| started.elapsed() >= timeout)
        {
            Some(ScanTermination::Timeout)
        } else {
            None
        };
        if let Some(reason) = reason {
            let code = termination_code(reason);
            let _ = self
                .0
                .compare_exchange(0, code, Ordering::AcqRel, Ordering::Acquire);
        }
        termination_from_code(self.0.load(Ordering::Acquire))
    }
}

const fn termination_code(reason: ScanTermination) -> u8 {
    match reason {
        ScanTermination::Timeout => 1,
        ScanTermination::Cancelled => 2,
        ScanTermination::MaxEntries | ScanTermination::MaxTotalBytes => 0,
    }
}

const fn termination_from_code(code: u8) -> Option<ScanTermination> {
    match code {
        1 => Some(ScanTermination::Timeout),
        2 => Some(ScanTermination::Cancelled),
        _ => None,
    }
}

fn record_limit_skip(
    inspected: &mut InspectedFiles,
    relative: String,
    reason: ScanTermination,
    options: &ScanOptions,
) {
    if options.evidence == EvidenceMode::Complete {
        inspected.skipped.push(SkippedEntry {
            relative,
            kind: SkipKind::ScanLimit,
            detail: Some(format!("{reason:?}")),
        });
    }
}

fn binary_skip(relative: String) -> SkippedEntry {
    SkippedEntry {
        relative,
        kind: SkipKind::Binary,
        detail: None,
    }
}

fn record_binary_skip(inspected: &mut InspectedFiles, relative: String, options: &ScanOptions) {
    if options.evidence == EvidenceMode::Complete {
        inspected.skipped.push(binary_skip(relative));
    }
}

fn record_concurrent_modification(
    inspected: &mut InspectedFiles,
    relative: String,
    options: &ScanOptions,
) -> Result<()> {
    if options.walk.error_policy == ErrorPolicy::Abort {
        return Err(Error::concurrent_modification(relative));
    }
    let message = "file changed while the scan was reading it".to_owned();
    if options.evidence == EvidenceMode::Complete {
        inspected.skipped.push(SkippedEntry {
            relative: relative.clone(),
            kind: SkipKind::ConcurrentModification,
            detail: Some(message.clone()),
        });
    }
    inspected.warnings.push(ScanWarning {
        relative: Some(relative),
        message,
    });
    Ok(())
}

fn record_io_error(
    inspected: &mut InspectedFiles,
    file: &ScannedFile,
    operation: &str,
    source: io::Error,
    options: &ScanOptions,
) -> Result<()> {
    if options.walk.error_policy == ErrorPolicy::Abort {
        return Err(Error::io(&file.absolute, source));
    }
    let message = format!("{operation}: {source}");
    if options.evidence == EvidenceMode::Complete {
        inspected.skipped.push(SkippedEntry {
            relative: file.relative.clone(),
            kind: SkipKind::IoError,
            detail: Some(message.clone()),
        });
    }
    inspected.warnings.push(ScanWarning {
        relative: Some(file.relative.clone()),
        message,
    });
    Ok(())
}

#[cfg(test)]
#[path = "content/tests.rs"]
mod tests;
