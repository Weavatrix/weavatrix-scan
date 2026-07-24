use crate::cache::ScanCache;
use crate::config::{EvidenceMode, ScanOptions};
use crate::content::inspect_files;
use crate::error::{Error, Result};
use crate::ignore::RepositoryMatcher;
use crate::parallel::WalkControl;
use crate::parallel::dynamic::{self, BatchControl};
use crate::report::ScanReport;
use crate::scan_finalize::{RevisionBuilder, finalize_report, sort_report_evidence};
use crate::scan_limits::{ScanRuntime, apply_total_bytes_limit};
use crate::scan_stream::{ScanSink, ScanSinkControl, ScanStreamReport};
use crate::walker::{ErrorPolicy, Walker};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

mod entry;

use entry::{process_entry, record_walk_error, walker_error_into_scan_error};

pub struct Scanner {
    root: PathBuf,
    options: ScanOptions,
}

impl Scanner {
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            options: ScanOptions::default(),
        }
    }

    #[must_use]
    pub fn options(mut self, options: ScanOptions) -> Self {
        self.options = options;
        self
    }

    /// Scans the configured repository root.
    ///
    /// # Errors
    ///
    /// Returns an error when the root cannot be canonicalized/read, or when a
    /// local error occurs under `ErrorPolicy::Abort`.
    pub fn scan(self) -> Result<ScanReport> {
        scan_repository_with_options(&self.root, &self.options, None)
    }

    /// Scans while reusing strong hashes from an older persistent report when
    /// file identity, size and timestamps are unchanged.
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`Self::scan`]. Reports from another root or
    /// reports without file-version evidence are scanned without cache reuse.
    pub fn scan_incremental(self, previous: &ScanReport) -> Result<ScanReport> {
        let cache = previous.to_cache();
        scan_repository_with_options(&self.root, &self.options, Some(&cache))
    }

    /// Scans while reusing a compact, versioned local cache.
    ///
    /// Incompatible versions and caches belonging to another canonical root
    /// are safely ignored.
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`Self::scan`].
    pub fn scan_cached(self, cache: &ScanCache) -> Result<ScanReport> {
        scan_repository_with_options(&self.root, &self.options, Some(cache))
    }

    /// Inspects and emits the deterministic manifest under synchronous
    /// backpressure without retaining selected file records.
    ///
    /// The sink is invoked in normalized relative-path order. Returning
    /// [`ScanSinkControl::Stop`] stops inspection and marks the stream summary
    /// incomplete.
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`Self::scan`].
    pub fn scan_into<S>(self, mut sink: S) -> Result<ScanStreamReport>
    where
        S: ScanSink,
    {
        let (mut report, runtime) = discover_repository_with_options(&self.root, &self.options)?;
        sort_report_evidence(&mut report);
        let files = std::mem::take(&mut report.files);
        let mut revision = RevisionBuilder::new(&report);
        let mut selected = 0_u64;
        let mut emitted = 0_u64;
        let mut stopped = false;
        for file in files {
            let inspected = inspect_files(vec![file], &self.options, runtime.started, None)?;
            report.skipped.extend(inspected.skipped);
            if !inspected.warnings.is_empty() {
                report.complete = false;
                report.warnings.extend(inspected.warnings);
            }
            report.cache.reused_hashes = report
                .cache
                .reused_hashes
                .saturating_add(inspected.cache.reused_hashes);
            report.cache.content_reads = report
                .cache
                .content_reads
                .saturating_add(inspected.cache.content_reads);
            if let Some(reason) = inspected.termination {
                report.terminate(reason);
            }
            for file in inspected.files {
                revision.push(&file);
                selected = selected.saturating_add(1);
                emitted = emitted.saturating_add(1);
                if sink.on_file(&file) == ScanSinkControl::Stop {
                    stopped = true;
                    report.complete = false;
                    break;
                }
            }
            if stopped || report.termination.is_some() {
                break;
            }
        }
        sort_report_evidence(&mut report);
        report.revision = revision.finish(&report);
        report.finish_recording();
        Ok(ScanStreamReport {
            root: report.root,
            selected,
            emitted,
            stopped,
            skipped: report.skipped,
            warnings: report.warnings,
            ignore_sources: report.ignore_sources,
            revision: report.revision,
            complete: report.complete,
            termination: report.termination,
            portable: report.portable,
            cache: report.cache,
        })
    }
}

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
    let (mut report, runtime) = discover_repository_with_options(root, options)?;
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

fn discover_repository_with_options(
    root: &Path,
    options: &ScanOptions,
) -> Result<(ScanReport, ScanRuntime)> {
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
        discover_parallel(&canonical, options, report, matcher, runtime)?
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
                if process_entry(&entry, options, &mut report, &mut matcher)? {
                    walker.skip_current_dir();
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
        &cancellation,
        move |entries, errors| {
            let mut state = visitor_state
                .lock()
                .expect("parallel scanner state is not poisoned");
            let mut controls = Vec::with_capacity(entries.len());
            let mut quit = state.error.is_some();
            for entry in entries {
                if quit {
                    controls.push(WalkControl::Quit);
                    continue;
                }
                if let Some(reason) = state.runtime.before_next(&visitor_options) {
                    state.report.terminate(reason);
                    controls.push(WalkControl::Quit);
                    quit = true;
                    continue;
                }
                state.runtime.record_entry();
                let ParallelDiscovery {
                    report, matcher, ..
                } = &mut *state;
                match process_entry(entry, &visitor_options, report, matcher) {
                    Ok(true) => controls.push(WalkControl::Skip),
                    Ok(false) => controls.push(WalkControl::Continue),
                    Err(error) => {
                        state.error = Some(error);
                        controls.push(WalkControl::Quit);
                        quit = true;
                    }
                }
            }
            for error in errors {
                if quit {
                    break;
                }
                if let Some(reason) = state.runtime.before_next(&visitor_options) {
                    state.report.terminate(reason);
                    quit = true;
                    break;
                }
                state.runtime.record_entry();
                if visitor_options.walk.error_policy == ErrorPolicy::Continue {
                    record_walk_error(error, &visitor_root, &mut state.report);
                }
            }
            BatchControl {
                entries: controls,
                quit,
            }
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
