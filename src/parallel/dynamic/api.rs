use super::{
    Arc, AssertUnwindSafe, AtomicU64, BatchControl, CancellationToken, ErrorPolicy, FileIdentity,
    Ordering, ParallelRuntime, ParallelVisitReport, Path, WalkControl, WalkEntry, WalkError,
    WalkEvent, WalkOptions, catch_unwind, initial_state, mpsc, prepare_root, resume_unwind,
    run_batched, schedule_error, stream_batched_serial, stream_worker, visit_batched_serial,
    worker, worker_count,
};

pub(crate) fn visit<F>(
    root: &Path,
    options: WalkOptions,
    parallelism: usize,
    runtime: &ParallelRuntime,
    skip_stdout: Option<FileIdentity>,
    cancellation: &CancellationToken,
    visitor: F,
) -> Result<ParallelVisitReport, WalkError>
where
    F: for<'entry> Fn(WalkEvent<'entry>) -> WalkControl + Send + Sync + 'static,
{
    let skipped = Arc::new(AtomicU64::new(0));
    let visitor_skipped = Arc::clone(&skipped);
    let mut report = visit_batched(
        root,
        options,
        parallelism,
        runtime,
        cancellation,
        move |entries, errors| {
            let mut controls = Vec::with_capacity(entries.len());
            let mut quit = false;
            for entry in entries {
                if crate::parallel::matches_stdout(entry, skip_stdout) {
                    visitor_skipped.fetch_add(1, Ordering::Relaxed);
                    controls.push(WalkControl::Continue);
                    continue;
                }
                let control = visitor(WalkEvent::Entry(entry));
                controls.push(control);
                quit |= control == WalkControl::Quit;
            }
            for error in errors {
                quit |= visitor(WalkEvent::Error(error)) == WalkControl::Quit;
            }
            BatchControl {
                entries: controls,
                quit,
            }
        },
    )?;
    report.visited = report
        .visited
        .saturating_sub(skipped.load(Ordering::Relaxed));
    Ok(report)
}

pub(crate) fn visit_batched<F>(
    root: &Path,
    options: WalkOptions,
    parallelism: usize,
    runtime: &ParallelRuntime,
    cancellation: &CancellationToken,
    visitor: F,
) -> Result<ParallelVisitReport, WalkError>
where
    F: Fn(&[WalkEntry], &[WalkError]) -> BatchControl + Send + Sync + 'static,
{
    if runtime.is_worker_thread() {
        return visit_batched_serial(root, options, cancellation, visitor);
    }
    run_batched(root, options, parallelism, runtime, cancellation, visitor)
}

pub(crate) fn stream_batched<F>(
    root: &Path,
    options: WalkOptions,
    parallelism: usize,
    runtime: &ParallelRuntime,
    cancellation: &CancellationToken,
    visitor: F,
) -> Result<ParallelVisitReport, WalkError>
where
    F: Fn(Vec<WalkEntry>, &[WalkError]) -> bool + Send + Sync + 'static,
{
    if runtime.is_worker_thread() {
        return stream_batched_serial(root, options, cancellation, visitor);
    }
    let (parallel_root, root_file_system, root_entry) = prepare_root(root, options)?;
    let root_can_descend = root_entry.skip_reason().is_none();
    let root_identity = root_entry.directory_identity();
    let root_visible = root_entry.depth() >= options.min_depth;
    let mut visited = u64::from(root_visible);
    let mut keep_going = true;
    if !cancellation.is_cancelled() && root_visible {
        keep_going = visitor(vec![root_entry], &[]);
    }
    let descend = !cancellation.is_cancelled() && root_can_descend && keep_going;
    if !descend {
        return Ok(ParallelVisitReport {
            visited,
            errors: Vec::new(),
            quit: !keep_going,
            cancelled: cancellation.is_cancelled(),
        });
    }

    let shared = initial_state(&parallel_root, visited, root_identity);
    let visitor = Arc::new(visitor);
    let cancellation = cancellation.clone();
    let worker_count = worker_count(runtime, parallelism, options.max_open);
    let (sender, receiver) = mpsc::channel();
    for submitted in 0..worker_count {
        let worker_shared = Arc::clone(&shared);
        let visitor = Arc::clone(&visitor);
        let root = Arc::clone(&parallel_root);
        let cancellation = cancellation.clone();
        let sender = sender.clone();
        if let Err(source) = runtime.try_execute(move || {
            let outcome = catch_unwind(AssertUnwindSafe(|| {
                stream_worker(
                    &worker_shared,
                    &root,
                    root_file_system,
                    options,
                    &cancellation,
                    visitor.as_ref(),
                );
            }));
            if outcome.is_err() {
                worker::abort_after_panic(&worker_shared);
            }
            let _ = sender.send(outcome);
        }) {
            worker::abort_after_submit_error(&shared);
            for _ in 0..submitted {
                let _ = receiver.recv();
            }
            return Err(schedule_error(parallel_root.as_path(), source));
        }
    }
    drop(sender);
    let mut panic = None;
    for outcome in receiver {
        if let Err(payload) = outcome
            && panic.is_none()
        {
            panic = Some(payload);
        }
    }
    if let Some(payload) = panic {
        resume_unwind(payload);
    }

    let shared = Arc::try_unwrap(shared)
        .ok()
        .expect("dynamic traversal workers released shared state");
    let mut state = shared
        .state
        .into_inner()
        .expect("dynamic traversal state is not poisoned");
    if options.error_policy == ErrorPolicy::Abort && !state.errors.is_empty() {
        return Err(state.errors.remove(0));
    }
    visited = state.visited;
    Ok(ParallelVisitReport {
        visited,
        errors: state.errors,
        quit: state.quit,
        cancelled: cancellation.is_cancelled(),
    })
}
