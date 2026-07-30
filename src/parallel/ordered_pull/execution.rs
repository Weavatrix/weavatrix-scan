use super::{
    Arc, CancellationToken, DirectoryFrame, DirectoryTask, ErrorPolicy, HashMap, HashSet,
    OrderedScheduler, ParallelWalker, PullBatch, SyncSender, VecDeque, Walker, mpsc,
};

pub(super) fn ordered_serial(
    walker: &ParallelWalker,
    cancellation: &CancellationToken,
    sender: &SyncSender<PullBatch>,
) {
    let error_policy = walker.options.error_policy;
    let skip_stdout = walker.skip_stdout;
    let mut options = walker.options.normalized();
    options.error_policy = ErrorPolicy::Continue;
    let mut walker = match Walker::with_options(&walker.root, options) {
        Ok(walker) => walker,
        Err(error) => {
            let _ = sender.send(vec![Err(error)]);
            return;
        }
    };
    while !cancellation.is_cancelled() {
        let Some(item) = walker.next() else {
            break;
        };
        let abort = error_policy == ErrorPolicy::Abort && item.is_err();
        let visible = match item.as_ref() {
            Ok(entry) => !super::super::matches_stdout(entry, skip_stdout),
            Err(_) => true,
        };
        if (visible && sender.send(vec![item]).is_err()) || abort {
            break;
        }
    }
}

pub(super) fn ordered_parallel(
    walker: &ParallelWalker,
    cancellation: &CancellationToken,
    sender: &SyncSender<PullBatch>,
) {
    let options = walker.options.normalized();
    let mut root_options = options;
    root_options.min_depth = 0;
    root_options.error_policy = ErrorPolicy::Continue;
    let mut root_walker = match Walker::with_options(&walker.root, root_options) {
        Ok(walker) => walker,
        Err(error) => {
            let _ = sender.send(vec![Err(error)]);
            return;
        }
    };
    let root_file_system = root_walker.root_file_system;
    let root = Arc::clone(&root_walker.root);
    let root_entry = match root_walker
        .next()
        .expect("a validated root yields one entry")
    {
        Ok(entry) => entry,
        Err(error) => {
            let _ = sender.send(vec![Err(error)]);
            return;
        }
    };
    let root_identity = root_entry.directory_identity();
    let root_can_descend = root_entry.is_dir() && root_entry.skip_reason().is_none();
    if root_entry.depth() >= options.min_depth
        && !super::super::matches_stdout(&root_entry, walker.skip_stdout)
        && sender.send(vec![Ok(root_entry)]).is_err()
    {
        return;
    }
    if !root_can_descend || cancellation.is_cancelled() {
        return;
    }

    let (result_sender, result_receiver) = mpsc::channel();
    let mut scheduler = OrderedScheduler {
        root: Arc::clone(&root),
        root_file_system,
        options,
        cancellation: cancellation.clone(),
        runtime: walker.runtime.clone(),
        limit: super::super::requested_workers(
            &walker.runtime,
            walker.parallelism,
            options.max_open,
        ),
        next_id: 1,
        queued: VecDeque::new(),
        outstanding: 0,
        ready: HashMap::new(),
        result_sender,
        result_receiver,
        schedule_error: None,
    };
    let mut ancestors = HashSet::new();
    if let Some(identity) = root_identity {
        ancestors.insert(identity);
    }
    scheduler.queued.push_back(DirectoryTask {
        id: 0,
        path: root.as_ref().clone(),
        depth: 0,
        identity: root_identity,
        ancestors: Arc::new(ancestors),
    });
    scheduler.refill();
    let root_batch = match scheduler.wait_for(0) {
        Ok(Some(batch)) => batch,
        Ok(None) => return,
        Err(error) => {
            let _ = sender.send(vec![Err(error)]);
            return;
        }
    };
    let mut frames = vec![scheduler.prepare_frame(root_batch)];

    drive_scheduler(
        &mut scheduler,
        &mut frames,
        cancellation,
        sender,
        options.min_depth,
        options.error_policy,
        walker.skip_stdout,
    );
}

#[allow(clippy::too_many_arguments)]
fn drive_scheduler(
    scheduler: &mut OrderedScheduler,
    frames: &mut Vec<DirectoryFrame>,
    cancellation: &CancellationToken,
    sender: &SyncSender<PullBatch>,
    min_depth: usize,
    error_policy: ErrorPolicy,
    skip_stdout: Option<crate::report::FileIdentity>,
) {
    while !cancellation.is_cancelled() {
        let Some(frame) = frames.last_mut() else {
            break;
        };
        let Some(prepared) = frame.items.next() else {
            frames.pop();
            continue;
        };
        let child = prepared.child;
        let visible = prepared
            .item
            .as_ref()
            .map_or(true, |entry| entry.depth() >= min_depth);
        let abort = error_policy == ErrorPolicy::Abort && prepared.item.is_err();
        let visible = visible
            && match prepared.item.as_ref() {
                Ok(entry) => !super::super::matches_stdout(entry, skip_stdout),
                Err(_) => true,
            };
        if visible && sender.send(vec![prepared.item]).is_err() {
            scheduler.cancel_and_drain();
            return;
        }
        if abort {
            scheduler.cancel_and_drain();
            return;
        }
        if let Some(child) = child {
            let batch = match scheduler.wait_for(child) {
                Ok(Some(batch)) => batch,
                Ok(None) => return,
                Err(error) => {
                    let _ = sender.send(vec![Err(error)]);
                    return;
                }
            };
            frames.push(scheduler.prepare_frame(batch));
        }
    }
    scheduler.cancel_and_drain();
}
