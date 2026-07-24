use crate::config::{EvidenceMode, ScanOptions};
use crate::error::{Error, Result};
use crate::file_version::reusable;
use crate::report::{
    ScanCacheStats, ScanReport, ScanTermination, ScanWarning, ScannedFile, SkipKind, SkippedEntry,
};
use crate::walker::ErrorPolicy;
use inspect::Inspection;
use std::collections::HashMap;
use std::io;
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::Instant;

mod inspect;

pub(crate) struct InspectedFiles {
    pub(crate) files: Vec<ScannedFile>,
    pub(crate) skipped: Vec<SkippedEntry>,
    pub(crate) warnings: Vec<ScanWarning>,
    pub(crate) termination: Option<ScanTermination>,
    pub(crate) cache: ScanCacheStats,
}

pub(crate) fn inspect_files(
    files: Vec<ScannedFile>,
    options: &ScanOptions,
    started: Instant,
    previous: Option<&ScanReport>,
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
    let cache = previous.map_or_else(HashMap::new, |report| {
        report
            .files
            .iter()
            .map(|file| (file.relative.as_str(), file))
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
        }
        Ok(inspected)
    })
}

fn inspect_chunk(
    files: Vec<ScannedFile>,
    options: &ScanOptions,
    started: Instant,
    stop: &InspectionStop,
    cache: &HashMap<&str, &ScannedFile>,
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
        if reuse_cached(&mut file, options, cached) {
            inspected.cache.reused_hashes = inspected.cache.reused_hashes.saturating_add(1);
            inspected.files.push(file);
            continue;
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

fn reuse_cached(
    current: &mut ScannedFile,
    options: &ScanOptions,
    previous: Option<&ScannedFile>,
) -> bool {
    let Some(previous) = previous else {
        return false;
    };
    let Some(hash) = previous
        .content_hash
        .as_ref()
        .filter(|hash| hash.starts_with("sha256:"))
    else {
        return false;
    };
    if current.bytes != previous.bytes
        || !reusable(&previous.version, &current.version)
        || (options.detect_binary_files && !previous.binary_checked)
    {
        return false;
    }
    current.content_hash = Some(hash.clone());
    current.binary_checked = previous.binary_checked;
    true
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
