use super::{
    ParallelRuntime, PathBuf, Result, ScanCache, ScanOptions, ScanReport, Scanner, compact, paths,
    scan_repository_with_runtime, watch_update,
};
use crate::watch::WatchPlan;

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

    /// Collects sorted relative paths without building a portable manifest.
    ///
    /// Selection follows [`Self::scan`]: ignore files, standard skips,
    /// extensions, and hidden-file policy. The walk does not collect sizes,
    /// hashes, revision, or skip evidence, and it does not apply
    /// `max_file_bytes`.
    ///
    /// Prefer this over mapping `files` out of a metadata-only report when
    /// the consumer only needs names.
    ///
    /// # Errors
    ///
    /// Returns the same root and I/O errors as [`Self::scan`]. Cancel and
    /// timeout surface as interrupted I/O.
    pub fn scan_paths(self) -> Result<Vec<String>> {
        paths::scan_repository_paths_with_runtime(&self.root, &self.options, &self.runtime)
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
    /// Safe file-only plans re-match and inspect only changed paths, drop
    /// deleted paths and their confirmed descendants, merge unchanged
    /// manifest evidence, and recompute the revision. A policy/descriptor
    /// mismatch, ignore-source change, structural event, or unsafe path
    /// falls back to a complete scan.
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`Self::scan`].
    pub fn scan_watch_plan(self, previous: &ScanReport, plan: &WatchPlan) -> Result<ScanReport> {
        Ok(self.scan_watch_plan_detailed(previous, plan)?.report)
    }

    /// Applies a watcher plan and reports whether the fast path was kept.
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`Self::scan`].
    pub fn scan_watch_plan_detailed(
        self,
        previous: &ScanReport,
        plan: &WatchPlan,
    ) -> Result<crate::WatchUpdate> {
        watch_update::scan_watch_plan(&self.root, &self.options, previous, plan, &self.runtime)
    }
}
