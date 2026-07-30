use super::{
    CompactContentEvidence, CompactScannedFile, Error, ErrorPolicy, FileVersion, PreparedDiscovery,
    RepositoryMatcher, Result, ScanOptions, ScanReport, ScanRuntime, VisitedFiles, Walker, mpsc,
    process_entry_with, record_walk_error, walker_error_into_scan_error,
};
use crate::control::CancellationToken;
use crate::report::ScanTermination;
use crate::walk_types::RootSymlinkPolicy;

pub(super) fn prepare_discovery(
    root: &std::path::Path,
    options: &ScanOptions,
) -> Result<PreparedDiscovery> {
    if options.walk.root_symlink_policy == RootSymlinkPolicy::Reject {
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
    Ok(PreparedDiscovery {
        evidence: ScanReport::new(
            canonical.clone(),
            options.evidence == crate::EvidenceMode::Complete,
        ),
        matcher: RepositoryMatcher::with_options(&canonical, options)?,
        runtime: ScanRuntime::new(),
        root: canonical,
    })
}

pub(super) fn stream_discover_serial(
    mut prepared: PreparedDiscovery,
    options: &ScanOptions,
    sender: &mpsc::SyncSender<(u64, CompactScannedFile)>,
) -> Result<(ScanReport, ScanRuntime, u64)> {
    let mut walker = Walker::with_options(&prepared.root, options.walk_options())
        .map_err(walker_error_into_scan_error)?;
    let mut discovered = 0_u64;
    loop {
        if let Some(reason) = prepared.runtime.before_next(options) {
            prepared.evidence.terminate(reason);
            break;
        }
        let Some(item) = walker.next() else {
            break;
        };
        prepared.runtime.record_entry();
        match item {
            Ok(entry) => {
                let mut selected = None;
                let skip = process_entry_with(
                    &entry,
                    options,
                    &mut prepared.evidence,
                    &prepared.matcher,
                    None,
                    |_path, relative, bytes, version| {
                        selected = Some(content_candidate(relative, bytes, version));
                    },
                )?;
                if let Some(file) = selected {
                    if !send_candidate(sender, discovered, file, options, &mut prepared.evidence)? {
                        break;
                    }
                    discovered = discovered.saturating_add(1);
                }
                if skip {
                    walker.skip_current_dir();
                } else if entry.is_dir() {
                    prepared.matcher.prepare_directory(entry.path())?;
                }
            }
            Err(error) if options.walk.error_policy == ErrorPolicy::Abort => {
                return Err(walker_error_into_scan_error(error));
            }
            Err(error) => record_walk_error(&error, &prepared.root, &mut prepared.evidence),
        }
    }
    Ok(finish_stream_discovery(prepared, discovered))
}

pub(super) fn content_candidate(
    relative: String,
    bytes: u64,
    version: FileVersion,
) -> CompactScannedFile {
    CompactScannedFile {
        relative: relative.into_boxed_str(),
        bytes,
        content: Some(Box::new(CompactContentEvidence {
            content_hash: None,
            content_fingerprint: None,
            version,
            binary_checked: false,
        })),
    }
}

pub(super) fn send_candidate(
    sender: &mpsc::SyncSender<(u64, CompactScannedFile)>,
    sequence: u64,
    file: CompactScannedFile,
    options: &ScanOptions,
    evidence: &mut ScanReport,
) -> Result<bool> {
    if options
        .cancellation
        .as_ref()
        .is_some_and(CancellationToken::is_cancelled)
    {
        evidence.terminate(ScanTermination::Cancelled);
        return Ok(false);
    }
    if sender.send((sequence, file)).is_ok() {
        return Ok(true);
    }
    if options
        .cancellation
        .as_ref()
        .is_some_and(CancellationToken::is_cancelled)
    {
        evidence.terminate(ScanTermination::Cancelled);
        Ok(false)
    } else {
        Err(Error::io(
            &evidence.root,
            std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "content workers stopped before traversal completed",
            ),
        ))
    }
}

pub(super) fn finish_stream_discovery(
    mut prepared: PreparedDiscovery,
    discovered: u64,
) -> (ScanReport, ScanRuntime, u64) {
    prepared.evidence.ignore_sources = prepared.matcher.sources().to_vec();
    prepared.evidence.portable = prepared.matcher.portable();
    if !prepared.matcher.warnings().is_empty() {
        prepared.evidence.complete = false;
        prepared
            .evidence
            .warnings
            .extend_from_slice(prepared.matcher.warnings());
    }
    (prepared.evidence, prepared.runtime, discovered)
}

type StreamWorkerOutcome = std::thread::Result<Result<VisitedFiles>>;

pub(super) fn collect_stream_outcomes(
    receiver: &mpsc::Receiver<(usize, StreamWorkerOutcome)>,
    scheduled: usize,
    root: &std::path::Path,
) -> Result<Vec<VisitedFiles>> {
    let mut outcomes = Vec::with_capacity(scheduled);
    for _ in 0..scheduled {
        outcomes.push(receiver.recv().map_err(|source| {
            Error::io(
                root,
                std::io::Error::new(std::io::ErrorKind::BrokenPipe, source),
            )
        })?);
    }
    outcomes.sort_unstable_by_key(|(worker_index, _)| *worker_index);
    let mut reports = Vec::with_capacity(scheduled);
    let mut first_error = None;
    let mut first_panic = None;
    for (_, outcome) in outcomes {
        match outcome {
            Ok(Ok(report)) => reports.push(report),
            Ok(Err(error)) if first_error.is_none() => first_error = Some(error),
            Err(panic) if first_panic.is_none() => first_panic = Some(panic),
            Ok(Err(_)) | Err(_) => {}
        }
    }
    if let Some(panic) = first_panic {
        std::panic::resume_unwind(panic);
    }
    if let Some(error) = first_error {
        return Err(error);
    }
    Ok(reports)
}
