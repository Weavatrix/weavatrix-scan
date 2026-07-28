use super::entry::{process_entry_with, record_walk_error, walker_error_into_scan_error};
use crate::config::{EvidenceMode, ScanOptions};
use crate::content::inspect_files;
use crate::error::{Error, Result};
use crate::hash::FingerprintHasher;
use crate::ignore::RepositoryMatcher;
use crate::parallel::dynamic;
use crate::report::{
    CompactContentEvidence, CompactScanReport, CompactScannedFile, ScanReport, ScannedFile,
    SkipKind, SkippedEntry,
};
use crate::runtime::ParallelRuntime;
use crate::scan_limits::ScanRuntime;
use crate::walker::{ErrorPolicy, Walker};
use std::path::Path;
use std::sync::{Arc, Mutex};

/// Scans a repository into a root-shared compact manifest.
///
/// # Errors
///
/// Returns an error when the root cannot be resolved/read or when local I/O
/// fails under `ErrorPolicy::Abort`.
pub fn scan_repository_compact(root: impl AsRef<Path>) -> Result<CompactScanReport> {
    scan_repository_compact_with_runtime(
        root.as_ref(),
        &ScanOptions::default(),
        &ParallelRuntime::global(),
    )
}

pub(super) fn scan_repository_compact_with_runtime(
    root: &Path,
    options: &ScanOptions,
    parallel_runtime: &ParallelRuntime,
) -> Result<CompactScanReport> {
    let (mut evidence, mut files, runtime) = discover_compact(root, options, parallel_runtime)?;
    files.sort_unstable_by(|left, right| left.relative.cmp(&right.relative));
    apply_total_bytes_limit(&mut evidence, &mut files, options);

    if options.hash_file_contents || options.detect_binary_files {
        let canonical_root = evidence.root.clone();
        files = inspect_compact(
            &canonical_root,
            files,
            options,
            runtime.started,
            &mut evidence,
        )?;
    }
    sort_evidence(&mut evidence);
    let revision = compact_revision(&evidence, &files);
    evidence.finish_recording();
    Ok(CompactScanReport {
        root: evidence.root,
        files,
        skipped: evidence.skipped,
        warnings: evidence.warnings,
        ignore_sources: evidence.ignore_sources,
        revision,
        complete: evidence.complete,
        termination: evidence.termination,
        portable: evidence.portable,
        cache: evidence.cache,
    })
}

pub(super) fn discover_compact(
    root: &Path,
    options: &ScanOptions,
    parallel_runtime: &ParallelRuntime,
) -> Result<(ScanReport, Vec<CompactScannedFile>, ScanRuntime)> {
    if options.walk.root_symlink_policy == crate::RootSymlinkPolicy::Reject {
        let metadata = std::fs::symlink_metadata(root).map_err(|source| Error::io(root, source))?;
        if metadata.file_type().is_symlink() {
            return Err(Error::io(
                root,
                std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "root symlink rejected by policy",
                ),
            ));
        }
    }
    let canonical = root
        .canonicalize()
        .map_err(|source| Error::io(root, source))?;
    if !canonical.is_dir() {
        return Err(Error::InvalidRoot(canonical));
    }
    let evidence = ScanReport::new(
        canonical.clone(),
        options.evidence == EvidenceMode::Complete,
    );
    let matcher = RepositoryMatcher::with_options(&canonical, options)?;
    let runtime = ScanRuntime::new();
    let (mut evidence, files, matcher, runtime) = if options.uses_parallel_traversal() {
        discover_parallel(
            &canonical,
            options,
            evidence,
            matcher,
            runtime,
            parallel_runtime,
        )?
    } else {
        discover_serial(&canonical, options, evidence, matcher, runtime)?
    };
    evidence.ignore_sources = matcher.sources().to_vec();
    evidence.portable = matcher.portable();
    if !matcher.warnings().is_empty() {
        evidence.complete = false;
        evidence.warnings.extend_from_slice(matcher.warnings());
    }
    if evidence.termination.is_none()
        && let Some(reason) = runtime.external_termination(options)
    {
        evidence.terminate(reason);
    }
    Ok((evidence, files, runtime))
}

fn discover_serial(
    canonical: &Path,
    options: &ScanOptions,
    mut evidence: ScanReport,
    mut matcher: RepositoryMatcher,
    mut runtime: ScanRuntime,
) -> Result<(
    ScanReport,
    Vec<CompactScannedFile>,
    RepositoryMatcher,
    ScanRuntime,
)> {
    let mut files = Vec::new();
    let mut walker = Walker::with_options(canonical, options.walk_options())
        .map_err(walker_error_into_scan_error)?;
    loop {
        if let Some(reason) = runtime.before_next(options) {
            evidence.terminate(reason);
            break;
        }
        let Some(item) = walker.next() else {
            break;
        };
        runtime.record_entry();
        match item {
            Ok(entry) => {
                let skip = process_entry_with(
                    &entry,
                    options,
                    &mut evidence,
                    &matcher,
                    None,
                    |_path, relative, bytes, version| {
                        files.push(compact_file(relative, bytes, version, options));
                    },
                )?;
                if skip {
                    walker.skip_current_dir();
                } else if entry.is_dir() {
                    matcher.prepare_directory(entry.path())?;
                }
            }
            Err(error) if options.walk.error_policy == ErrorPolicy::Abort => {
                return Err(walker_error_into_scan_error(error));
            }
            Err(error) => record_walk_error(&error, canonical, &mut evidence),
        }
    }
    Ok((evidence, files, matcher, runtime))
}

struct ParallelCompactDiscovery {
    evidence: ScanReport,
    files: Vec<CompactScannedFile>,
    matcher: RepositoryMatcher,
    runtime: ScanRuntime,
    error: Option<Error>,
}

#[allow(clippy::too_many_lines)]
fn discover_parallel(
    canonical: &Path,
    options: &ScanOptions,
    evidence: ScanReport,
    matcher: RepositoryMatcher,
    runtime: ScanRuntime,
    parallel_runtime: &ParallelRuntime,
) -> Result<(
    ScanReport,
    Vec<CompactScannedFile>,
    RepositoryMatcher,
    ScanRuntime,
)> {
    let state = Arc::new(Mutex::new(ParallelCompactDiscovery {
        evidence,
        files: Vec::new(),
        matcher,
        runtime,
        error: None,
    }));
    let visitor_state = Arc::clone(&state);
    let visitor_options = options.clone();
    let visitor_root = canonical.to_path_buf();
    let cancellation = options.cancellation.clone().unwrap_or_default();
    let traversal = dynamic::visit_batched(
        canonical,
        options.walk_options(),
        options.traversal_workers(),
        parallel_runtime,
        &cancellation,
        move |entries, errors| {
            let mut state = visitor_state
                .lock()
                .expect("compact scanner state is not poisoned");
            let ParallelCompactDiscovery {
                evidence,
                files,
                matcher,
                runtime,
                error: scan_error,
            } = &mut *state;
            super::batch::process_parallel_batch(
                entries,
                errors,
                &visitor_root,
                &visitor_options,
                evidence,
                matcher,
                runtime,
                scan_error,
                files,
                |_path, relative, bytes, version| {
                    compact_file(relative, bytes, version, &visitor_options)
                },
            )
        },
    );
    if let Err(error) = traversal {
        return Err(walker_error_into_scan_error(error));
    }
    let state = Arc::try_unwrap(state)
        .ok()
        .expect("compact scanner visitor released shared state");
    let mut state = state
        .into_inner()
        .expect("compact scanner state is not poisoned");
    if let Some(error) = state.error.take() {
        return Err(error);
    }
    Ok((state.evidence, state.files, state.matcher, state.runtime))
}

fn compact_file(
    relative: String,
    bytes: u64,
    version: crate::FileVersion,
    options: &ScanOptions,
) -> CompactScannedFile {
    CompactScannedFile {
        relative: relative.into_boxed_str(),
        bytes,
        content: (options.hash_file_contents || options.detect_binary_files).then(|| {
            Box::new(CompactContentEvidence {
                content_hash: None,
                content_fingerprint: None,
                version,
                binary_checked: false,
            })
        }),
    }
}

fn inspect_compact(
    root: &Path,
    files: Vec<CompactScannedFile>,
    options: &ScanOptions,
    started: std::time::Instant,
    evidence: &mut ScanReport,
) -> Result<Vec<CompactScannedFile>> {
    const CHUNK_SIZE: usize = 4_096;
    let mut iterator = files.into_iter();
    let mut inspected_files = Vec::new();
    loop {
        let chunk = iterator
            .by_ref()
            .take(CHUNK_SIZE)
            .map(|file| {
                let content = file
                    .content
                    .expect("content scan compact entry retains discovery evidence");
                ScannedFile {
                    absolute: root.join(file.relative.as_ref()),
                    relative: file.relative.into(),
                    bytes: file.bytes,
                    content_hash: content.content_hash.map(Into::into),
                    content_fingerprint: content.content_fingerprint.map(Into::into),
                    version: content.version,
                    binary_checked: content.binary_checked,
                }
            })
            .collect::<Vec<_>>();
        if chunk.is_empty() {
            break;
        }
        let inspected = inspect_files(chunk, options, started, None)?;
        inspected_files.extend(inspected.files.into_iter().map(|file| CompactScannedFile {
            relative: file.relative.into_boxed_str(),
            bytes: file.bytes,
            content: Some(Box::new(CompactContentEvidence {
                content_hash: file.content_hash.map(String::into_boxed_str),
                content_fingerprint: file.content_fingerprint.map(String::into_boxed_str),
                version: file.version,
                binary_checked: file.binary_checked,
            })),
        }));
        evidence.skipped.extend(inspected.skipped);
        evidence.warnings.extend(inspected.warnings);
        evidence.cache.reused_hashes = evidence
            .cache
            .reused_hashes
            .saturating_add(inspected.cache.reused_hashes);
        evidence.cache.content_reads = evidence
            .cache
            .content_reads
            .saturating_add(inspected.cache.content_reads);
        evidence.cache.fingerprint_reads = evidence
            .cache
            .fingerprint_reads
            .saturating_add(inspected.cache.fingerprint_reads);
        if let Some(reason) = inspected.termination {
            evidence.terminate(reason);
        }
        if !evidence.warnings.is_empty() {
            evidence.complete = false;
        }
        if let Some(reason) = evidence.termination {
            if options.evidence == EvidenceMode::Complete {
                evidence
                    .skipped
                    .extend(iterator.by_ref().map(|file| SkippedEntry {
                        relative: file.relative.into(),
                        kind: SkipKind::ScanLimit,
                        detail: Some(format!("content inspection stopped: {reason:?}")),
                    }));
            }
            break;
        }
    }
    Ok(inspected_files)
}

pub(super) fn apply_total_bytes_limit(
    evidence: &mut ScanReport,
    files: &mut Vec<CompactScannedFile>,
    options: &ScanOptions,
) {
    let Some(limit) = options.limits.max_total_bytes else {
        return;
    };
    let mut total = 0_u64;
    let keep = files
        .iter()
        .position(|file| {
            let next = total.saturating_add(file.bytes);
            if next > limit {
                true
            } else {
                total = next;
                false
            }
        })
        .unwrap_or(files.len());
    if keep == files.len() {
        return;
    }
    let truncated = files.split_off(keep);
    if options.evidence == EvidenceMode::Complete {
        evidence
            .skipped
            .extend(truncated.into_iter().map(|file| SkippedEntry {
                relative: file.relative.into(),
                kind: SkipKind::ScanLimit,
                detail: Some(format!("maximum selected bytes: {limit}")),
            }));
    }
    evidence.terminate(crate::ScanTermination::MaxTotalBytes);
}

pub(super) fn sort_evidence(report: &mut ScanReport) {
    report.skipped.sort_unstable_by(|left, right| {
        left.relative
            .cmp(&right.relative)
            .then_with(|| left.kind.cmp(&right.kind))
            .then_with(|| left.detail.cmp(&right.detail))
    });
    report.warnings.sort_unstable_by(|left, right| {
        left.relative
            .cmp(&right.relative)
            .then_with(|| left.message.cmp(&right.message))
    });
    report.ignore_sources.sort_unstable_by(|left, right| {
        left.kind
            .cmp(&right.kind)
            .then_with(|| left.location.cmp(&right.location))
            .then_with(|| left.content_hash.cmp(&right.content_hash))
    });
    report.ignore_sources.dedup();
}

pub(super) fn compact_revision(evidence: &ScanReport, files: &[CompactScannedFile]) -> String {
    let mut revision = FingerprintHasher::new();
    for source in &evidence.ignore_sources {
        revision.write(format!("{:?}", source.kind).as_bytes());
        revision.write(&[0]);
        revision.write(source.location.as_bytes());
        revision.write(&[0]);
        revision.write(source.content_hash.as_bytes());
        revision.write(&[0xfe]);
    }
    for file in files {
        revision.write(file.relative.as_bytes());
        revision.write(&[0]);
        revision.write(file.content_hash().unwrap_or("").as_bytes());
        revision.write(&[0xff]);
    }
    revision.write(&[u8::from(evidence.portable)]);
    if let Some(termination) = evidence.termination {
        revision.write(format!("{termination:?}").as_bytes());
    }
    revision.finish()
}
