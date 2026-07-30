use super::{
    ChangedContentVisitOutcome, CompactScanReport, ContentVisitControl, ContentVisitEvent,
    ContentVisitMode, ContentVisitReport, FinishedContentVisit, Result, Scanner,
    apply_total_bytes_limit, discover_compact, finish_content_report, run_workers,
    visit_changed_content_plan, visit_content_direct,
};
use crate::watch::WatchPlan;

impl Scanner {
    /// Visits selected file bytes once with bounded parallelism.
    ///
    /// Ignore rules, file limits, path safety, binary detection, content
    /// hashing, and revision evidence use the same scanner configuration. A
    /// worker-local visitor is created by `factory`; callback order is
    /// intentionally concurrent. Every event carries a monotonic work
    /// sequence plus its root and normalized relative path; use the paths for
    /// deterministic cross-run ordering.
    ///
    /// `ContentVisitControl::SkipFile` stops delivering chunks for the current
    /// file. The scanner still finishes reading when hashing or binary
    /// detection requires complete evidence. `ContentVisitControl::Quit`
    /// cooperatively cancels every worker.
    ///
    /// # Errors
    ///
    /// Returns root, traversal, content I/O, or worker-submission failures
    /// according to the configured error policy.
    ///
    /// # Panics
    ///
    /// Propagates a panic from the factory or visitor after active workers
    /// observe cancellation.
    pub fn visit_content<Factory, Visitor>(self, factory: Factory) -> Result<ContentVisitReport>
    where
        Factory: Fn(usize) -> Visitor + Send + Sync + 'static,
        Visitor:
            for<'event> FnMut(ContentVisitEvent<'event>) -> ContentVisitControl + Send + 'static,
    {
        self.visit_content_with_root(0, factory)
    }

    /// Visits selected file bytes once and returns the exact compact manifest
    /// produced by those same verified reads.
    ///
    /// This additive API lets parsers consume bytes and retain incremental scan
    /// evidence without changing the long-standing [`ContentVisitReport`]
    /// construction contract used by existing scanner consumers.
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`Self::visit_content`].
    ///
    /// # Panics
    ///
    /// Propagates callback panics like [`Self::visit_content`].
    pub fn visit_content_manifest<Factory, Visitor>(
        self,
        factory: Factory,
    ) -> Result<CompactScanReport>
    where
        Factory: Fn(usize) -> Visitor + Send + Sync + 'static,
        Visitor:
            for<'event> FnMut(ContentVisitEvent<'event>) -> ContentVisitControl + Send + 'static,
    {
        self.visit_content_with_root_mode_finished(0, ContentVisitMode::Revision, factory)
            .map(FinishedContentVisit::into_manifest)
    }

    /// Visits selected bytes without retaining a selected-file manifest or
    /// computing a revision.
    ///
    /// Typed skip evidence is still retained when `EvidenceMode::Complete` is
    /// configured. Use `selected_files_only()` as well for constant-memory
    /// summary reporting.
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`Self::visit_content`].
    ///
    /// # Panics
    ///
    /// Propagates callback panics like [`Self::visit_content`].
    pub fn visit_content_streaming<Factory, Visitor>(
        self,
        factory: Factory,
    ) -> Result<ContentVisitReport>
    where
        Factory: Fn(usize) -> Visitor + Send + Sync + 'static,
        Visitor:
            for<'event> FnMut(ContentVisitEvent<'event>) -> ContentVisitControl + Send + 'static,
    {
        self.visit_content_with_root_mode(0, ContentVisitMode::Streaming, factory)
    }

    /// Visits only safe changed-file paths from a watcher plan.
    ///
    /// No directory traversal occurs. Plans that can affect directory
    /// structure or file-selection rules return
    /// [`ChangedContentVisitOutcome::FullRescanRequired`] before invoking the
    /// factory. Removed paths are returned separately.
    ///
    /// # Errors
    ///
    /// Returns root, matcher, content I/O, or worker-submission failures
    /// according to the configured error policy.
    ///
    /// # Panics
    ///
    /// Propagates a panic from the factory or visitor after active workers
    /// observe cancellation.
    pub fn visit_changed_content<Factory, Visitor>(
        self,
        plan: &WatchPlan,
        factory: Factory,
    ) -> Result<ChangedContentVisitOutcome>
    where
        Factory: Fn(usize) -> Visitor + Send + Sync + 'static,
        Visitor:
            for<'event> FnMut(ContentVisitEvent<'event>) -> ContentVisitControl + Send + 'static,
    {
        visit_changed_content_plan(self, plan, ContentVisitMode::Revision, factory)
    }

    /// Visits only changed-file bytes without retaining their compact manifest
    /// or computing a subset revision.
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`Self::visit_changed_content`].
    pub fn visit_changed_content_streaming<Factory, Visitor>(
        self,
        plan: &WatchPlan,
        factory: Factory,
    ) -> Result<ChangedContentVisitOutcome>
    where
        Factory: Fn(usize) -> Visitor + Send + Sync + 'static,
        Visitor:
            for<'event> FnMut(ContentVisitEvent<'event>) -> ContentVisitControl + Send + 'static,
    {
        visit_changed_content_plan(self, plan, ContentVisitMode::Streaming, factory)
    }

    pub(crate) fn visit_content_with_root<Factory, Visitor>(
        self,
        root_index: usize,
        factory: Factory,
    ) -> Result<ContentVisitReport>
    where
        Factory: Fn(usize) -> Visitor + Send + Sync + 'static,
        Visitor:
            for<'event> FnMut(ContentVisitEvent<'event>) -> ContentVisitControl + Send + 'static,
    {
        self.visit_content_with_root_mode(root_index, ContentVisitMode::Revision, factory)
    }

    pub(crate) fn visit_content_with_root_mode<Factory, Visitor>(
        self,
        root_index: usize,
        mode: ContentVisitMode,
        factory: Factory,
    ) -> Result<ContentVisitReport>
    where
        Factory: Fn(usize) -> Visitor + Send + Sync + 'static,
        Visitor:
            for<'event> FnMut(ContentVisitEvent<'event>) -> ContentVisitControl + Send + 'static,
    {
        self.visit_content_with_root_mode_finished(root_index, mode, factory)
            .map(|finished| finished.report)
    }

    fn visit_content_with_root_mode_finished<Factory, Visitor>(
        self,
        root_index: usize,
        mode: ContentVisitMode,
        factory: Factory,
    ) -> Result<FinishedContentVisit>
    where
        Factory: Fn(usize) -> Visitor + Send + Sync + 'static,
        Visitor:
            for<'event> FnMut(ContentVisitEvent<'event>) -> ContentVisitControl + Send + 'static,
    {
        if self.options.limits.max_total_bytes.is_none()
            && self.options.content_discovery == crate::ContentDiscoveryMode::Streaming
            && !self.runtime.is_worker_thread()
        {
            return visit_content_direct(self, root_index, mode, factory);
        }
        let mut discovery_options = self.options.clone();
        discovery_options.detect_binary_files = true;
        let (mut evidence, mut files, scan_runtime) =
            discover_compact(&self.root, &discovery_options, &self.runtime)?;
        files.sort_unstable_by(|left, right| left.relative.cmp(&right.relative));
        apply_total_bytes_limit(&mut evidence, &mut files, &self.options);
        let discovered = u64::try_from(files.len()).unwrap_or(u64::MAX);

        let cancellation = self.options.cancellation.clone().unwrap_or_default();
        let mut visit_options = self.options.clone();
        visit_options.cancellation = Some(cancellation.clone());
        let workers = visit_options
            .content_visit_worker_count(files.len())
            .min(self.runtime.parallelism())
            .max(1);
        let worker_reports = run_workers(
            evidence.root.clone(),
            files,
            visit_options,
            &scan_runtime,
            &self.runtime,
            workers,
            root_index,
            mode,
            factory,
        )?;

        Ok(finish_content_report(
            evidence,
            discovered,
            worker_reports,
            mode,
        ))
    }
}
