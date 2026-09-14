use crate::config::ScanOptions;
use crate::control::CancellationToken;
use crate::error::{Error, Result};
use crate::portable_report::PortableScannedFile;
use crate::report::ScanReport;
use crate::runtime::ParallelRuntime;
use crate::scanner::{Scanner, scan_repository_with_runtime};
use crate::watch::WatchPlan;
use crate::watch_reason::{FullRescanReason, WatchUpdateReason};
use std::path::{Path, PathBuf};

/// Owns one scan snapshot and applies later watch plans against it.
///
/// Incremental apply still walks the retained manifest in `O(N)` to drop
/// invalidated paths and recompute revision. It does not claim `O(k)` cost.
/// Each snapshot replacement increments [`Self::generation`].
pub struct ScanSession {
    root: PathBuf,
    options: ScanOptions,
    runtime: ParallelRuntime,
    report: ScanReport,
    generation: u64,
    last_reason: Option<WatchUpdateReason>,
}

impl ScanSession {
    /// Scans `root` and retains the resulting snapshot.
    ///
    /// One-shot cancellation is not stored on the session after open.
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`Scanner::scan`].
    pub fn open(root: impl Into<PathBuf>, mut options: ScanOptions) -> Result<Self> {
        let root = root.into();
        let runtime = ParallelRuntime::global();
        let report = scan_repository_with_runtime(&root, &options, None, &runtime)?;
        options.cancellation = None;
        Ok(Self {
            root,
            options,
            runtime,
            report,
            generation: 1,
            last_reason: None,
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

    /// Snapshot generation used by [`Self::files_page`].
    #[must_use]
    pub const fn generation(&self) -> u64 {
        self.generation
    }

    /// Reason of the last apply or rescan, if any update has run.
    #[must_use]
    pub const fn last_update_reason(&self) -> Option<WatchUpdateReason> {
        self.last_reason
    }

    #[must_use]
    pub fn into_report(self) -> ScanReport {
        self.report
    }

    /// Converts one page of the current snapshot, or [`Error::StaleSnapshot`].
    ///
    /// # Errors
    ///
    /// Returns [`Error::StaleSnapshot`] when `generation` does not match.
    pub fn files_page(
        &self,
        generation: u64,
        offset: usize,
        limit: usize,
    ) -> Result<Vec<PortableScannedFile>> {
        if generation != self.generation {
            return Err(Error::stale_snapshot(generation, self.generation));
        }
        Ok(self.report.portable_files_page(offset, limit))
    }

    /// Replaces the snapshot with a complete scan using the current policy.
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`Scanner::scan`].
    pub fn rescan(&mut self) -> Result<&ScanReport> {
        self.install(
            scan_repository_with_runtime(&self.root, &self.options, None, &self.runtime)?,
            WatchUpdateReason::FullRescan(FullRescanReason::StructuralChange),
        );
        Ok(&self.report)
    }

    /// Applies a watcher plan, falling back to a complete scan when required.
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`Scanner::scan_watch_plan`].
    pub fn apply_watch_plan(&mut self, plan: &WatchPlan) -> Result<&ScanReport> {
        self.apply_watch_plan_with_cancellation(plan, None)
    }

    /// Applies a watcher plan with cancellation that lasts only for this update.
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`Scanner::scan_watch_plan`].
    pub fn apply_watch_plan_with_cancellation(
        &mut self,
        plan: &WatchPlan,
        cancellation: Option<CancellationToken>,
    ) -> Result<&ScanReport> {
        let mut options = self.options.clone();
        options.cancellation = cancellation;
        let update = Scanner::new(&self.root)
            .options(options)
            .runtime(self.runtime.clone())
            .scan_watch_plan_detailed(&self.report, plan)?;
        self.install(update.report, update.reason);
        Ok(&self.report)
    }

    fn install(&mut self, report: ScanReport, reason: WatchUpdateReason) {
        self.report = report;
        self.last_reason = Some(reason);
        self.generation = self.generation.saturating_add(1);
    }
}
