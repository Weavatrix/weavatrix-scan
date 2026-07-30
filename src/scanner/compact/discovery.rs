use super::{
    Arc, CompactScanReport, CompactScannedFile, Error, ErrorPolicy, EvidenceMode, Mutex,
    ParallelRuntime, Path, RepositoryMatcher, Result, ScanOptions, ScanReport, ScanRuntime, Walker,
    apply_total_bytes_limit, compact_file, compact_revision, dynamic, inspect_compact,
    process_entry_with, record_walk_error, sort_evidence, walker_error_into_scan_error,
};

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

pub(crate) fn scan_repository_compact_with_runtime(
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

pub(crate) fn discover_compact(
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
            super::super::batch::process_parallel_batch(
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
