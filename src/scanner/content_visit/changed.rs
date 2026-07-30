use super::{
    Arc, AssertUnwindSafe, ChangedContentVisitOutcome, ChangedContentVisitReport,
    ContentVisitControl, ContentVisitEvent, ContentVisitMode, ContentWorkerContext, Error,
    FinishedContentVisit, Mutex, PathBuf, RepositoryMatcher, Result, ScanReport, ScanRuntime,
    Scanner, VisitedFiles, apply_total_bytes_limit, catch_unwind, collect_stream_outcomes,
    content_candidate, finish_content_report, finish_stream_discovery, mpsc, prepare_discovery,
    run_workers, stream_discover_serial, visit_files,
};
use crate::watch::WatchPlan;

pub(super) fn visit_changed_content_plan<Factory, Visitor>(
    scanner: Scanner,
    plan: &WatchPlan,
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
            .any(|relative| !super::super::watch_update::is_safe_relative(relative))
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
        match super::super::watch_update::changed_candidate(
            &prepared.root,
            &relative,
            &options,
            &mut prepared.matcher,
            &mut prepared.evidence,
        )? {
            super::super::watch_update::ChangedPath::Candidate(file) => {
                let file = *file;
                files.push(content_candidate(file.relative, file.bytes, file.version));
            }
            super::super::watch_update::ChangedPath::MissingOrSkipped => {}
            super::super::watch_update::ChangedPath::NeedsFullScan => {
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
            content: finish_content_report(evidence, discovered, worker_reports, mode).report,
            removed,
        },
    )))
}

pub(super) struct PreparedDiscovery {
    pub(super) root: PathBuf,
    pub(super) evidence: ScanReport,
    pub(super) matcher: RepositoryMatcher,
    pub(super) runtime: ScanRuntime,
}

pub(super) fn visit_content_direct<Factory, Visitor>(
    scanner: Scanner,
    root_index: usize,
    mode: ContentVisitMode,
    factory: Factory,
) -> Result<FinishedContentVisit>
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
                run_direct_worker(
                    worker_index,
                    worker_receiver.as_ref(),
                    worker_factory.as_ref(),
                    &worker_options,
                    worker_root.as_ref(),
                    started,
                    root_index,
                    mode,
                )
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

#[allow(clippy::too_many_arguments)]
fn run_direct_worker<Factory, Visitor>(
    worker_index: usize,
    receiver: &Mutex<mpsc::Receiver<(u64, super::CompactScannedFile)>>,
    factory: &Factory,
    options: &super::ScanOptions,
    root: &PathBuf,
    started: std::time::Instant,
    root_index: usize,
    mode: ContentVisitMode,
) -> Result<VisitedFiles>
where
    Factory: Fn(usize) -> Visitor + Send + Sync + 'static,
    Visitor: for<'event> FnMut(ContentVisitEvent<'event>) -> ContentVisitControl + Send + 'static,
{
    let mut visitor = factory(worker_index);
    let mut buffer = vec![0_u8; 64 * 1024].into_boxed_slice();
    let mut aggregate = VisitedFiles::empty(0);
    loop {
        let batch = {
            let receiver = receiver
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
                    Err(mpsc::TryRecvError::Empty | mpsc::TryRecvError::Disconnected) => break,
                }
            }
            batch
        };
        let visited = visit_files(
            batch,
            options,
            started,
            ContentWorkerContext {
                root,
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
}
