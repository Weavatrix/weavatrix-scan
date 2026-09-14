use crate::config::ScanOptions;
use crate::error::Result;
use crate::report::ScanReport;
use crate::runtime::ParallelRuntime;
use crate::scanner::{Scanner, scan_repository_with_runtime};
use crate::watch::WatchPlan;
use std::path::{Path, PathBuf};

/// Owns one scan snapshot and applies later watch plans against it.
///
/// Incremental apply still walks the retained manifest in `O(N)` to drop
/// invalidated paths and recompute revision. It does not claim `O(k)` cost.
pub struct ScanSession {
    root: PathBuf,
    options: ScanOptions,
    runtime: ParallelRuntime,
    report: ScanReport,
}

impl ScanSession {
    /// Scans `root` and retains the resulting snapshot.
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`Scanner::scan`].
    pub fn open(root: impl Into<PathBuf>, options: ScanOptions) -> Result<Self> {
        let root = root.into();
        let runtime = ParallelRuntime::global();
        let report = scan_repository_with_runtime(&root, &options, None, &runtime)?;
        Ok(Self {
            root,
            options,
            runtime,
            report,
        })
    }

    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    #[must_use]
    pub const fn report(&self) -> &ScanReport {
        &self.report
    }

    #[must_use]
    pub fn into_report(self) -> ScanReport {
        self.report
    }

    /// Replaces the snapshot with a complete scan using the current policy.
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`Scanner::scan`].
    pub fn rescan(&mut self) -> Result<&ScanReport> {
        self.report = scan_repository_with_runtime(&self.root, &self.options, None, &self.runtime)?;
        Ok(&self.report)
    }

    /// Applies a watcher plan, falling back to a complete scan when required.
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`Scanner::scan_watch_plan`].
    pub fn apply_watch_plan(&mut self, plan: &WatchPlan) -> Result<&ScanReport> {
        self.report = Scanner::new(&self.root)
            .options(self.options.clone())
            .runtime(self.runtime.clone())
            .scan_watch_plan(&self.report, plan)?;
        Ok(&self.report)
    }
}
