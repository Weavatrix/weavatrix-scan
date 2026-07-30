use super::{
    Arc, AssertUnwindSafe, CompactScannedFile, ContentVisitControl, ContentVisitEvent,
    ContentVisitMode, ContentWorkerContext, Error, Mutex, ParallelRuntime, PathBuf, Result,
    ScanOptions, ScanRuntime, VecDeque, VisitedFiles, catch_unwind, mpsc, visit_files,
};

#[allow(clippy::too_many_arguments)]
pub(super) fn run_workers<Factory, Visitor>(
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

    let workers = workers.min(indexed.len());
    let queue = Arc::new(Mutex::new(VecDeque::from(indexed)));
    let root = Arc::new(root);
    let options = Arc::new(options);
    let factory = Arc::new(factory);
    let (sender, receiver) = mpsc::channel();
    let mut scheduled = 0_usize;
    let mut schedule_error = None;
    for worker_index in 0..workers {
        let worker_queue = Arc::clone(&queue);
        let worker_root = Arc::clone(&root);
        let worker_options = Arc::clone(&options);
        let worker_factory = Arc::clone(&factory);
        let worker_sender = sender.clone();
        let started = scan_runtime.started;
        if let Err(source) = runtime.try_execute(move || {
            let outcome = catch_unwind(AssertUnwindSafe(|| {
                run_worker_queue(
                    worker_index,
                    &worker_queue,
                    worker_root.as_ref(),
                    &worker_options,
                    worker_factory.as_ref(),
                    started,
                    root_index,
                    mode,
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

    collect_worker_outcomes(
        &receiver,
        scheduled,
        root.as_ref(),
        &options,
        schedule_error,
    )
}

#[allow(clippy::too_many_arguments)]
fn run_worker_queue<Factory, Visitor>(
    worker_index: usize,
    queue: &Mutex<VecDeque<(u64, CompactScannedFile)>>,
    root: &PathBuf,
    options: &ScanOptions,
    factory: &Factory,
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
        let work = {
            let mut queue = queue
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            queue.pop_front()
        };
        let Some(work) = work else {
            break;
        };
        let visited = visit_files(
            std::iter::once(work),
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

type WorkerOutcome = std::thread::Result<Result<VisitedFiles>>;

fn collect_worker_outcomes(
    receiver: &mpsc::Receiver<(usize, WorkerOutcome)>,
    scheduled: usize,
    root: &PathBuf,
    options: &ScanOptions,
    schedule_error: Option<std::io::Error>,
) -> Result<Vec<VisitedFiles>> {
    let mut outcomes = Vec::with_capacity(scheduled);
    for _ in 0..scheduled {
        let (worker_index, outcome) = receiver.recv().map_err(|source| {
            Error::io(
                root,
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
        return Err(Error::io(root, source));
    }
    outcomes
        .into_iter()
        .map(|(_, outcome)| outcome.expect("worker panic handled"))
        .collect()
}
