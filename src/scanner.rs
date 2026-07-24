use crate::cache::ScanCache;
use crate::config::{EvidenceMode, ScanOptions};
use crate::content::inspect_files;
use crate::error::{Error, Result};
use crate::ignore::RepositoryMatcher;
use crate::parallel::WalkControl;
use crate::parallel::dynamic::{self, BatchControl};
use crate::report::ScanReport;
use crate::runtime::ParallelRuntime;
use crate::scan_finalize::finalize_report;
use crate::scan_limits::{ScanRuntime, apply_total_bytes_limit};
use crate::walker::{ErrorPolicy, Walker};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

mod compact;
mod entry;
mod stream;
mod watch_update;

use entry::{process_entry, record_walk_error, walker_error_into_scan_error};

pub use compact::scan_repository_compact;

pub struct Scanner {
    root: PathBuf,
    options: ScanOptions,
    runtime: ParallelRuntime,
}

impl Scanner {
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            options: ScanOptions::default(),
            runtime: ParallelRuntime::global(),
        }
    }

    #[must_use]
    pub fn options(mut self, options: ScanOptions) -> Self {
        self.options = options;
        self
    }

    /// Selects the executor used by parallel discovery.
    #[must_use]
    pub fn runtime(mut self, runtime: ParallelRuntime) -> Self {
        self.runtime = runtime;
        self
    }

    /// Scans the configured repository root.
    ///
    /// # Errors
    ///
    /// Returns an error when the root cannot be canonicalized/read, or when a
    /// local error occurs under `ErrorPolicy::Abort`.
    pub fn scan(self) -> Result<ScanReport> {
        scan_repository_with_runtime(&self.root, &self.options, None, &self.runtime)
    }

    /// Scans into a compact manifest that stores the canonical root once.
    ///
    /// This is the preferred report for very large repositories when callers
    /// do not require an owned absolute path on every file record.
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`Self::scan`].
    pub fn scan_compact(self) -> Result<crate::CompactScanReport> {
        compact::scan_repository_compact_with_runtime(&self.root, &self.options, &self.runtime)
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
        scan_repository_with_runtime(&self.root, &self.options, Some(&cache), &self.runtime)
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
        scan_repository_with_runtime(&self.root, &self.options, Some(cache), &self.runtime)
    }

    /// Applies a watcher plan without traversing unchanged directories.
    ///
    /// Safe file-only plans re-match and inspect only changed paths, remove
    /// deleted paths, merge unchanged manifest evidence, and recompute the
    /// revision. Plans that can affect selection or directory structure fall
    /// back to a complete scan.
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`Self::scan`].
    pub fn scan_watch_plan(
        self,
        previous: &ScanReport,
        plan: &crate::WatchPlan,
    ) -> Result<ScanReport> {
        watch_update::scan_watch_plan(&self.root, &self.options, previous, plan, &self.runtime)
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

fn discover_repository_with_options(
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
