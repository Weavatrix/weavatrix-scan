use super::{
    Arc, Error, ErrorPolicy, EvidenceMode, Mutex, ParallelRuntime, Path, RepositoryMatcher, Result,
    ScanCache, ScanOptions, ScanReport, ScanRuntime, ScannedFile, Walker, apply_total_bytes_limit,
    batch, dynamic, finalize_report, inspect_files, process_entry, record_walk_error,
    walker_error_into_scan_error,
};

/// Scans a repository with default options.
///
/// # Errors
///
/// Returns an error when the root cannot be canonicalized/read, or when a local
/// error occurs under `ErrorPolicy::Abort`.
pub fn scan_repository(root: impl AsRef<Path>) -> Result<ScanReport> {
    scan_repository_with_options(root.as_ref(), &ScanOptions::default(), None)
}

pub(crate) fn scan_repository_with_options(
    root: &Path,
    options: &ScanOptions,
    previous: Option<&ScanCache>,
) -> Result<ScanReport> {
    scan_repository_with_runtime(root, options, previous, &ParallelRuntime::global())
}

pub(crate) fn scan_repository_with_runtime(
    root: &Path,
    options: &ScanOptions,
    previous: Option<&ScanCache>,
    parallel_runtime: &ParallelRuntime,
) -> Result<ScanReport> {
    let (mut report, runtime) = discover_repository_with_options(root, options, parallel_runtime)?;
    let previous = previous.filter(|cache| cache.is_compatible(&report.root));
    let inspected = inspect_files(
        std::mem::take(&mut report.files),
        options,
        runtime.started,
        previous,
    )?;
    report.files = inspected.files;
    report.cache = inspected.cache;
    report.skipped.extend(inspected.skipped);
    if let Some(reason) = inspected.termination {
        report.terminate(reason);
    }
    if !inspected.warnings.is_empty() {
        report.complete = false;
        report.warnings.extend(inspected.warnings);
    }
    finalize_report(&mut report);
    Ok(report)
}

pub(super) fn discover_repository_with_options(
    root: &Path,
    options: &ScanOptions,
    parallel_runtime: &ParallelRuntime,
) -> Result<(ScanReport, ScanRuntime)> {
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
    let report = ScanReport::new(
        canonical.clone(),
        options.evidence == EvidenceMode::Complete,
    );
    let matcher = RepositoryMatcher::with_options(&canonical, options)?;
    let runtime = ScanRuntime::new();
    let (mut report, matcher, runtime) = if options.uses_parallel_traversal() {
        discover_parallel(
            &canonical,
            options,
            report,
            matcher,
            runtime,
            parallel_runtime,
        )?
    } else {
        discover_serial(&canonical, options, report, matcher, runtime)?
    };

    report.ignore_sources = matcher.sources().to_vec();
    report.portable = matcher.portable();
    if !matcher.warnings().is_empty() {
        report.complete = false;
        report.warnings.extend_from_slice(matcher.warnings());
    }
    if report.termination.is_none()
        && let Some(reason) = runtime.external_termination(options)
    {
        report.terminate(reason);
    }
    apply_total_bytes_limit(&mut report, options);
    Ok((report, runtime))
}

fn discover_serial(
    canonical: &Path,
    options: &ScanOptions,
    mut report: ScanReport,
    mut matcher: RepositoryMatcher,
    mut runtime: ScanRuntime,
) -> Result<(ScanReport, RepositoryMatcher, ScanRuntime)> {
    let mut walker = Walker::with_options(canonical, options.walk_options())
        .map_err(walker_error_into_scan_error)?;
    loop {
        if let Some(reason) = runtime.before_next(options) {
            report.terminate(reason);
            break;
        }
        let Some(item) = walker.next() else {
            break;
        };
        runtime.record_entry();
        match item {
            Ok(entry) => {
                let skip = process_entry(&entry, options, &mut report, &matcher, None)?;
                if skip {
                    walker.skip_current_dir();
                } else if entry.is_dir() {
                    matcher.prepare_directory(entry.path())?;
                }
            }
            Err(error) if options.walk.error_policy == ErrorPolicy::Abort => {
                return Err(walker_error_into_scan_error(error));
            }
            Err(error) => record_walk_error(&error, canonical, &mut report),
        }
    }
    Ok((report, matcher, runtime))
}

struct ParallelDiscovery {
    report: ScanReport,
    matcher: RepositoryMatcher,
    runtime: ScanRuntime,
    error: Option<Error>,
}

fn discover_parallel(
    canonical: &Path,
    options: &ScanOptions,
    report: ScanReport,
    matcher: RepositoryMatcher,
    runtime: ScanRuntime,
    parallel_runtime: &ParallelRuntime,
) -> Result<(ScanReport, RepositoryMatcher, ScanRuntime)> {
    let state = Arc::new(Mutex::new(ParallelDiscovery {
        report,
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
                .expect("parallel scanner state is not poisoned");
            let ParallelDiscovery {
                report,
                matcher,
                runtime,
                error: scan_error,
            } = &mut *state;
            let mut files = std::mem::take(&mut report.files);
            let control = batch::process_parallel_batch(
                entries,
                errors,
                &visitor_root,
                &visitor_options,
                report,
                matcher,
                runtime,
                scan_error,
                &mut files,
                |path, relative, bytes, version| ScannedFile {
                    absolute: path.to_path_buf(),
                    relative,
                    bytes,
                    content_hash: None,
                    content_fingerprint: None,
                    version,
                    binary_checked: false,
                },
            );
            report.files = files;
            control
        },
    );
    if let Err(error) = traversal {
        return Err(walker_error_into_scan_error(error));
    }
    let state = Arc::try_unwrap(state)
        .ok()
        .expect("parallel scanner visitor released shared state");
    let mut state = state
        .into_inner()
        .expect("parallel scanner state is not poisoned");
    if let Some(error) = state.error.take() {
        return Err(error);
    }
    Ok((state.report, state.matcher, state.runtime))
}
