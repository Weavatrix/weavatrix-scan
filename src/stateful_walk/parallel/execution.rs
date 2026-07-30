use super::{
    Arc, CancellationToken, DirectoryFrame, DirectoryTask, ErrorPolicy, HashMap, HashSet,
    OrderedStatefulScheduler, StatefulWalkBuilder, StatefulWalkEntry, SyncSender, VecDeque,
    WalkError, Walker, mpsc, requested_workers,
};

pub(super) fn run_serial<R, E>(
    builder: StatefulWalkBuilder<R, E>,
    cancellation: &CancellationToken,
    sender: &SyncSender<Result<StatefulWalkEntry<E>, WalkError>>,
) where
    R: Clone + Send + 'static,
    E: Default + Send + 'static,
{
    let walker = match builder.build() {
        Ok(walker) => walker,
        Err(error) => {
            let _ = sender.send(Err(error));
            return;
        }
    };
    for item in walker {
        if cancellation.is_cancelled() || sender.send(item).is_err() {
            break;
        }
    }
}

pub(super) fn run_parallel<R, E>(
    builder: StatefulWalkBuilder<R, E>,
    cancellation: &CancellationToken,
    sender: &SyncSender<Result<StatefulWalkEntry<E>, WalkError>>,
) where
    R: Clone + Send + 'static,
    E: Default + Send + 'static,
{
    let options = builder.options.normalized();
    let mut root_options = options;
    root_options.min_depth = 0;
    root_options.error_policy = ErrorPolicy::Continue;
    let mut root_walker = match Walker::with_options(&builder.root, root_options) {
        Ok(walker) => walker,
        Err(error) => {
            let _ = sender.send(Err(error));
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
            let _ = sender.send(Err(error));
            return;
        }
    };
    let root_identity = root_entry.directory_identity();
    let can_descend = root_entry.is_dir() && root_entry.skip_reason().is_none();
    if root_entry.depth() >= options.min_depth
        && sender
            .send(Ok(StatefulWalkEntry {
                read_children: can_descend,
                entry: root_entry,
                state: E::default(),
            }))
            .is_err()
    {
        return;
    }
    if !can_descend || cancellation.is_cancelled() {
        return;
    }

    let (result_sender, result_receiver) = mpsc::channel();
    let limit = requested_workers(&builder.runtime, builder.parallelism, options.max_open);
    let mut scheduler = OrderedStatefulScheduler {
        root: Arc::clone(&root),
        root_file_system,
        options,
        processor: builder.processor,
        cancellation: cancellation.clone(),
        runtime: builder.runtime,
        limit,
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
        read_state: builder.root_read_dir_state,
    });
    scheduler.refill();
    let root_batch = match scheduler.wait_for(0) {
        Ok(Some(batch)) => batch,
        Ok(None) => return,
        Err(error) => {
            let _ = sender.send(Err(error));
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
    );
}

fn drive_scheduler<R, E>(
    scheduler: &mut OrderedStatefulScheduler<R, E>,
    frames: &mut Vec<DirectoryFrame<E>>,
    cancellation: &CancellationToken,
    sender: &SyncSender<Result<StatefulWalkEntry<E>, WalkError>>,
    min_depth: usize,
    error_policy: ErrorPolicy,
) where
    R: Clone + Send + 'static,
    E: Default + Send + 'static,
{
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
        if visible && sender.send(prepared.item).is_err() {
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
                    let _ = sender.send(Err(error));
                    return;
                }
            };
            frames.push(scheduler.prepare_frame(batch));
        }
    }
    scheduler.cancel_and_drain();
}
