use super::Scanner;
use super::compact::{apply_total_bytes_limit, compact_revision, discover_compact, sort_evidence};
use super::entry::{process_entry_with, record_walk_error, walker_error_into_scan_error};
use crate::config::ScanOptions;
use crate::content::{ContentWorkerContext, VisitedFiles, visit_files};
use crate::content_visit::{
    ChangedContentVisitOutcome, ChangedContentVisitReport, ContentVisitControl, ContentVisitEvent,
    ContentVisitMode, ContentVisitReport,
};
use crate::error::{Error, Result};
use crate::ignore::RepositoryMatcher;
use crate::report::{CompactContentEvidence, CompactScannedFile, FileVersion, ScanReport};
use crate::runtime::ParallelRuntime;
use crate::scan_limits::ScanRuntime;
use crate::walker::{ErrorPolicy, Walker};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, mpsc};

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
        plan: &crate::WatchPlan,
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
        plan: &crate::WatchPlan,
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
        if self.options.limits.max_total_bytes.is_none() && !self.runtime.is_worker_thread() {
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

fn visit_changed_content_plan<Factory, Visitor>(
    scanner: Scanner,
    plan: &crate::WatchPlan,
    mode: ContentVisitMode,
    factory: Factory,
) -> Result<ChangedContentVisitOutcome>
where
    Factory: Fn(usize) -> Visitor + Send + Sync + 'static,
    Visitor: for<'event> FnMut(ContentVisitEvent<'event>) -> ContentVisitControl + Send + 'static,
{
    if plan.full_rescan
        || plan
            .invalidated()
            .any(|relative| !super::watch_update::is_safe_relative(relative))
    {
        return Ok(ChangedContentVisitOutcome::FullRescanRequired);
    }

    let mut options = scanner.options;
    let cancellation = options.cancellation.clone().unwrap_or_default();
    options.cancellation = Some(cancellation);
    let mut prepared = prepare_discovery(&scanner.root, &options)?;
    let mut changed = plan.changed.clone();
    changed.sort_unstable();
    changed.dedup();
    let mut files = Vec::with_capacity(changed.len());
    for relative in changed {
        if let Some(reason) = prepared.runtime.before_next(&options) {
            prepared.evidence.terminate(reason);
            break;
        }
        prepared.runtime.record_entry();
        match super::watch_update::changed_candidate(
            &prepared.root,
            &relative,
            &options,
            &mut prepared.matcher,
            &mut prepared.evidence,
        )? {
            super::watch_update::ChangedPath::Candidate(file) => {
                let file = *file;
                files.push(content_candidate(file.relative, file.bytes, file.version));
            }
            super::watch_update::ChangedPath::MissingOrSkipped => {}
            super::watch_update::ChangedPath::NeedsFullScan => {
                return Ok(ChangedContentVisitOutcome::FullRescanRequired);
            }
        }
    }
    files.sort_unstable_by(|left, right| left.relative.cmp(&right.relative));
    let selected = u64::try_from(files.len()).unwrap_or(u64::MAX);
    let (mut evidence, runtime, _) = finish_stream_discovery(prepared, selected);
    apply_total_bytes_limit(&mut evidence, &mut files, &options);
    let discovered = u64::try_from(files.len()).unwrap_or(u64::MAX);
    let workers = options
        .content_visit_worker_count(files.len())
        .min(scanner.runtime.parallelism())
        .max(1);
    let worker_reports = run_workers(
        evidence.root.clone(),
        files,
        options,
        &runtime,
        &scanner.runtime,
        workers,
        0,
        mode,
        factory,
    )?;
    let mut removed = plan.removed.clone();
    removed.sort_unstable();
    removed.dedup();
    Ok(ChangedContentVisitOutcome::Visited(Box::new(
        ChangedContentVisitReport {
            content: finish_content_report(evidence, discovered, worker_reports, mode),
            removed,
        },
    )))
}

struct PreparedDiscovery {
    root: PathBuf,
    evidence: ScanReport,
    matcher: RepositoryMatcher,
    runtime: ScanRuntime,
}

#[allow(clippy::too_many_lines)]
fn visit_content_direct<Factory, Visitor>(
    scanner: Scanner,
    root_index: usize,
    mode: ContentVisitMode,
    factory: Factory,
) -> Result<ContentVisitReport>
where
    Factory: Fn(usize) -> Visitor + Send + Sync + 'static,
    Visitor: for<'event> FnMut(ContentVisitEvent<'event>) -> ContentVisitControl + Send + 'static,
{
    let cancellation = scanner.options.cancellation.clone().unwrap_or_default();
    let mut options = scanner.options;
    options.cancellation = Some(cancellation.clone());
    let mut discovery_options = options.clone();
    discovery_options.detect_binary_files = true;
    let prepared = prepare_discovery(&scanner.root, &discovery_options)?;
    let root = Arc::new(prepared.root.clone());
    let started = prepared.runtime.started;
    let workers = options
        .content_visit_worker_count(usize::MAX)
        .min(scanner.runtime.parallelism())
        .max(1);
    let (sender, receiver) = mpsc::sync_channel(workers.saturating_mul(64).max(1));
    let receiver = Arc::new(Mutex::new(receiver));
    let factory = Arc::new(factory);
    let worker_options = Arc::new(options.clone());
    let (outcome_sender, outcome_receiver) = mpsc::channel();
    let mut scheduled = 0_usize;
    let mut schedule_error = None;
    for worker_index in 0..workers {
        let worker_receiver = Arc::clone(&receiver);
        let worker_factory = Arc::clone(&factory);
        let worker_options = Arc::clone(&worker_options);
        let worker_root = Arc::clone(&root);
        let worker_cancellation = cancellation.clone();
        let worker_outcome = outcome_sender.clone();
        if let Err(source) = scanner.runtime.try_execute(move || {
            let outcome = catch_unwind(AssertUnwindSafe(|| {
                let mut visitor = worker_factory(worker_index);
                let mut buffer = vec![0_u8; 64 * 1024].into_boxed_slice();
                let mut aggregate = VisitedFiles::empty(0);
                loop {
                    let batch = {
                        let receiver = worker_receiver
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner);
                        let Ok(first) = receiver.recv() else {
                            break;
                        };
                        let mut batch = Vec::with_capacity(32);
                        batch.push(first);
                        while batch.len() < 32 {
                            match receiver.try_recv() {
                                Ok(work) => batch.push(work),
                                Err(
                                    mpsc::TryRecvError::Empty | mpsc::TryRecvError::Disconnected,
                                ) => break,
                            }
                        }
                        batch
                    };
                    let visited = visit_files(
                        batch,
                        &worker_options,
                        started,
                        ContentWorkerContext {
                            root: worker_root.as_ref(),
                            root_index,
                            worker_index,
                            mode,
                        },
                        &mut buffer,
                        &mut visitor,
                    )?;
                    let stop = visited.visitor_quit || visited.evidence.termination.is_some();
                    aggregate.merge(visited);
                    if stop {
                        break;
                    }
                }
                Ok(aggregate)
            }));
            if outcome.as_ref().is_err() || outcome.as_ref().is_ok_and(std::result::Result::is_err)
            {
                worker_cancellation.cancel();
            }
            let _ = worker_outcome.send((worker_index, outcome));
        }) {
            schedule_error = Some(source);
            cancellation.cancel();
            break;
        }
        scheduled = scheduled.saturating_add(1);
    }
    drop(outcome_sender);
    drop(receiver);
    if schedule_error.is_none() {
        let discovery = stream_discover_serial(prepared, &discovery_options, &sender);
        if discovery.is_err() {
            cancellation.cancel();
        }
        drop(sender);
        let worker_reports = collect_stream_outcomes(&outcome_receiver, scheduled, root.as_ref())?;
        let (mut evidence, scan_runtime, discovered) = discovery?;
        if evidence.termination.is_none()
            && let Some(reason) = scan_runtime.external_termination(&options)
        {
            evidence.terminate(reason);
        }
        return Ok(finish_content_report(
            evidence,
            discovered,
            worker_reports,
            mode,
        ));
    }

    drop(sender);
    let worker_result = collect_stream_outcomes(&outcome_receiver, scheduled, root.as_ref());
    let _ = worker_result?;
    Err(Error::io(
        root.as_ref(),
        schedule_error.expect("content worker scheduling failed"),
    ))
}

fn prepare_discovery(root: &std::path::Path, options: &ScanOptions) -> Result<PreparedDiscovery> {
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

fn stream_discover_serial(
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
                    &mut prepared.matcher,
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

fn content_candidate(relative: String, bytes: u64, version: FileVersion) -> CompactScannedFile {
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

fn send_candidate(
    sender: &mpsc::SyncSender<(u64, CompactScannedFile)>,
    sequence: u64,
    file: CompactScannedFile,
    options: &ScanOptions,
    evidence: &mut ScanReport,
) -> Result<bool> {
    if options
        .cancellation
        .as_ref()
        .is_some_and(crate::CancellationToken::is_cancelled)
    {
        evidence.terminate(crate::ScanTermination::Cancelled);
        return Ok(false);
    }
    if sender.send((sequence, file)).is_ok() {
        return Ok(true);
    }
    if options
        .cancellation
        .as_ref()
        .is_some_and(crate::CancellationToken::is_cancelled)
    {
        evidence.terminate(crate::ScanTermination::Cancelled);
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

fn finish_stream_discovery(
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

fn collect_stream_outcomes(
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

fn finish_content_report(
    mut evidence: ScanReport,
    discovered: u64,
    worker_reports: Vec<VisitedFiles>,
    mode: ContentVisitMode,
) -> ContentVisitReport {
    let mut selected = Vec::new();
    let mut totals = VisitedFiles::empty(0);
    for mut worker in worker_reports {
        selected.append(&mut worker.files);
        totals.merge(worker);
    }
    evidence.skipped.extend(totals.evidence.skipped);
    evidence.warnings.extend(totals.evidence.warnings);
    evidence.termination = evidence.termination.or(totals.evidence.termination);
    evidence.cache = totals.evidence.cache;
    if totals.visitor_quit {
        evidence.complete = false;
        evidence.termination = Some(crate::ScanTermination::Cancelled);
    }
    if !evidence.warnings.is_empty() || evidence.termination.is_some() {
        evidence.complete = false;
    }
    sort_evidence(&mut evidence);
    let revision = if mode == ContentVisitMode::Revision {
        selected.sort_unstable_by(|left, right| left.1.relative.cmp(&right.1.relative));
        let files = selected
            .into_iter()
            .map(|(_, file)| file)
            .collect::<Vec<_>>();
        compact_revision(&evidence, &files)
    } else {
        String::new()
    };
    let stopped = evidence.termination.is_some();
    evidence.finish_recording();
    ContentVisitReport {
        mode,
        root: evidence.root,
        discovered,
        completed: totals.completed,
        opened: totals.opened,
        chunks: totals.chunks,
        bytes_read: totals.bytes_read,
        bytes_emitted: totals.bytes_emitted,
        consumer_skipped: totals.consumer_skipped,
        stopped,
        skipped: evidence.skipped,
        warnings: evidence.warnings,
        ignore_sources: evidence.ignore_sources,
        revision,
        complete: evidence.complete,
        termination: evidence.termination,
        portable: evidence.portable,
        cache: evidence.cache,
    }
}

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
fn run_workers<Factory, Visitor>(
    root: PathBuf,
    files: Vec<CompactScannedFile>,
    options: ScanOptions,
    scan_runtime: &ScanRuntime,
    runtime: &ParallelRuntime,
    workers: usize,
    root_index: usize,
    mode: ContentVisitMode,
    factory: Factory,
) -> Result<Vec<VisitedFiles>>
where
    Factory: Fn(usize) -> Visitor + Send + Sync + 'static,
    Visitor: for<'event> FnMut(ContentVisitEvent<'event>) -> ContentVisitControl + Send + 'static,
{
    if files.is_empty() {
        return Ok(Vec::new());
    }
    let indexed = files
        .into_iter()
        .enumerate()
        .map(|(sequence, file)| (u64::try_from(sequence).unwrap_or(u64::MAX), file))
        .collect::<Vec<_>>();
    if workers <= 1 || runtime.is_worker_thread() {
        let mut visitor = factory(0);
        let mut buffer = vec![0_u8; 64 * 1024].into_boxed_slice();
        return visit_files(
            indexed,
            &options,
            scan_runtime.started,
            ContentWorkerContext {
                root: &root,
                root_index,
                worker_index: 0,
                mode,
            },
            &mut buffer,
            &mut visitor,
        )
        .map(|report| vec![report]);
    }

    let chunk_size = indexed.len().div_ceil(workers);
    let mut indexed = indexed.into_iter();
    let mut chunks = Vec::with_capacity(workers);
    loop {
        let chunk = indexed.by_ref().take(chunk_size).collect::<Vec<_>>();
        if chunk.is_empty() {
            break;
        }
        chunks.push(chunk);
    }
    let root = Arc::new(root);
    let options = Arc::new(options);
    let factory = Arc::new(factory);
    let (sender, receiver) = mpsc::channel();
    let mut scheduled = 0_usize;
    let mut schedule_error = None;
    for (worker_index, chunk) in chunks.into_iter().enumerate() {
        let worker_root = Arc::clone(&root);
        let worker_options = Arc::clone(&options);
        let worker_factory = Arc::clone(&factory);
        let worker_sender = sender.clone();
        let started = scan_runtime.started;
        if let Err(source) = runtime.try_execute(move || {
            let outcome = catch_unwind(AssertUnwindSafe(|| {
                let mut visitor = worker_factory(worker_index);
                let mut buffer = vec![0_u8; 64 * 1024].into_boxed_slice();
                visit_files(
                    chunk,
                    &worker_options,
                    started,
                    ContentWorkerContext {
                        root: worker_root.as_ref(),
                        root_index,
                        worker_index,
                        mode,
                    },
                    &mut buffer,
                    &mut visitor,
                )
            }));
            let _ = worker_sender.send((worker_index, outcome));
        }) {
            options
                .cancellation
                .as_ref()
                .expect("content visit installs cancellation")
                .cancel();
            schedule_error = Some(source);
            break;
        }
        scheduled = scheduled.saturating_add(1);
    }
    drop(sender);

    let mut outcomes = Vec::with_capacity(scheduled);
    for _ in 0..scheduled {
        let (worker_index, outcome) = receiver.recv().map_err(|source| {
            Error::io(
                root.as_ref(),
                std::io::Error::new(std::io::ErrorKind::BrokenPipe, source),
            )
        })?;
        if outcome.as_ref().is_ok_and(std::result::Result::is_err) {
            options
                .cancellation
                .as_ref()
                .expect("content visit installs cancellation")
                .cancel();
        }
        outcomes.push((worker_index, outcome));
    }
    outcomes.sort_unstable_by_key(|(worker_index, _)| *worker_index);
    if let Some(index) = outcomes.iter().position(|(_, outcome)| outcome.is_err()) {
        let (_, outcome) = outcomes.swap_remove(index);
        let Err(panic) = outcome else {
            unreachable!("panicked worker outcome exists");
        };
        std::panic::resume_unwind(panic);
    }
    if let Some(source) = schedule_error {
        return Err(Error::io(root.as_ref(), source));
    }
    outcomes
        .into_iter()
        .map(|(_, outcome)| outcome.expect("worker panic handled"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    #[test]
    fn content_visit_is_reentrant_on_its_runtime() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "weavatrix-content-reentrant-{}-{nonce}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("value.rs"), "fn value() {}\n").unwrap();
        let runtime = ParallelRuntime::dedicated(1).unwrap();
        let nested_runtime = runtime.clone();
        let (sender, receiver) = mpsc::channel();
        runtime
            .try_execute(move || {
                let result = Scanner::new(&root)
                    .options(
                        ScanOptions::default()
                            .with_extensions(["rs"])
                            .selected_files_only()
                            .metadata_only(),
                    )
                    .runtime(nested_runtime)
                    .visit_content(|_| |_| ContentVisitControl::Continue)
                    .map(|report| report.completed);
                let _ = std::fs::remove_dir_all(root);
                sender.send(result).unwrap();
            })
            .unwrap();
        assert_eq!(
            receiver
                .recv_timeout(Duration::from_secs(5))
                .unwrap()
                .unwrap(),
            1
        );
    }
}
