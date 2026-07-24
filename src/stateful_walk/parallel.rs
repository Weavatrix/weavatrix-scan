use super::{DirectoryProcessor, StatefulWalkBuilder, StatefulWalkEntry};
use crate::control::CancellationToken;
use crate::walk_platform::{DirectoryIdentity, FileSystemId};
use crate::{
    ErrorPolicy, ParallelRuntime, WalkError, WalkOperation, WalkOptions, WalkSkipReason, Walker,
};
use std::any::Any;
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, SyncSender, sync_channel};
use std::thread::JoinHandle;

struct DirectoryTask<R> {
    id: u64,
    path: PathBuf,
    depth: usize,
    identity: Option<DirectoryIdentity>,
    ancestors: Arc<HashSet<DirectoryIdentity>>,
    read_state: R,
}

struct WorkerResult<R, E> {
    id: u64,
    outcome: Result<DirectoryBatch<R, E>, Box<dyn Any + Send>>,
}

struct DirectoryBatch<R, E> {
    entries: Vec<Result<StatefulWalkEntry<E>, WalkError>>,
    child_state: R,
    ancestors: Arc<HashSet<DirectoryIdentity>>,
}

struct PreparedItem<E> {
    item: Result<StatefulWalkEntry<E>, WalkError>,
    child: Option<u64>,
}

struct DirectoryFrame<E> {
    items: std::vec::IntoIter<PreparedItem<E>>,
}

struct OrderedStatefulScheduler<R, E> {
    root: Arc<PathBuf>,
    root_file_system: Option<FileSystemId>,
    options: WalkOptions,
    processor: Option<DirectoryProcessor<R, E>>,
    cancellation: CancellationToken,
    runtime: ParallelRuntime,
    limit: usize,
    next_id: u64,
    queued: VecDeque<DirectoryTask<R>>,
    outstanding: usize,
    ready: HashMap<u64, DirectoryBatch<R, E>>,
    result_sender: mpsc::Sender<WorkerResult<R, E>>,
    result_receiver: mpsc::Receiver<WorkerResult<R, E>>,
    schedule_error: Option<WalkError>,
}

/// Bounded stateful pull iterator with parallel directory processing and
/// strict deterministic depth-first output.
pub struct ParallelStatefulWalker<E> {
    receiver: Option<Receiver<Result<StatefulWalkEntry<E>, WalkError>>>,
    cancellation: CancellationToken,
    coordinator: Option<JoinHandle<()>>,
}

impl<E> ParallelStatefulWalker<E> {
    pub(super) fn start<R>(
        builder: StatefulWalkBuilder<R, E>,
        capacity: usize,
    ) -> Result<Self, WalkError>
    where
        R: Clone + Send + 'static,
        E: Default + Send + 'static,
    {
        let root = builder.root.clone();
        let use_serial = builder.runtime.is_worker_thread();
        let (sender, receiver) = sync_channel(capacity.max(1));
        let cancellation = CancellationToken::new();
        let coordinator_cancellation = cancellation.clone();
        let coordinator = std::thread::Builder::new()
            .name("weavatrix-scan-stateful".to_owned())
            .spawn(move || {
                if use_serial {
                    run_serial(builder, &coordinator_cancellation, &sender);
                } else {
                    run_parallel(builder, &coordinator_cancellation, &sender);
                }
            })
            .map_err(|source| WalkError::new(root, 0, WalkOperation::ScheduleWorker, source))?;
        Ok(Self {
            receiver: Some(receiver),
            cancellation,
            coordinator: Some(coordinator),
        })
    }

    fn join_coordinator(&mut self) {
        if let Some(coordinator) = self.coordinator.take() {
            coordinator
                .join()
                .expect("parallel stateful coordinator panicked");
        }
    }
}

impl<E> Iterator for ParallelStatefulWalker<E> {
    type Item = Result<StatefulWalkEntry<E>, WalkError>;

    fn next(&mut self) -> Option<Self::Item> {
        if let Ok(item) = self.receiver.as_ref()?.recv() {
            Some(item)
        } else {
            self.receiver.take();
            self.join_coordinator();
            None
        }
    }
}

impl<E> Drop for ParallelStatefulWalker<E> {
    fn drop(&mut self) {
        self.receiver.take();
        self.cancellation.cancel();
        self.join_coordinator();
    }
}

fn run_serial<R, E>(
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

#[allow(clippy::too_many_lines)]
fn run_parallel<R, E>(
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
            .map_or(true, |entry| entry.depth() >= options.min_depth);
        let abort = options.error_policy == ErrorPolicy::Abort && prepared.item.is_err();
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

impl<R, E> OrderedStatefulScheduler<R, E>
where
    R: Clone + Send + 'static,
    E: Default + Send + 'static,
{
    fn refill(&mut self) {
        while !self.cancellation.is_cancelled() && self.outstanding < self.limit {
            let Some(task) = self.queued.pop_front() else {
                break;
            };
            let root = Arc::clone(&self.root);
            let result_sender = self.result_sender.clone();
            let cancellation = self.cancellation.clone();
            let root_file_system = self.root_file_system;
            let options = self.options;
            let processor = self.processor.clone();
            let scheduled = self.runtime.try_execute(move || {
                let id = task.id;
                let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    read_directory(
                        &root,
                        root_file_system,
                        options,
                        &cancellation,
                        processor.as_ref(),
                        task,
                    )
                }));
                let _ = result_sender.send(WorkerResult { id, outcome });
            });
            match scheduled {
                Ok(()) => self.outstanding += 1,
                Err(source) => {
                    self.cancellation.cancel();
                    self.queued.clear();
                    self.schedule_error = Some(WalkError::new(
                        self.root.as_ref(),
                        0,
                        WalkOperation::ScheduleWorker,
                        source,
                    ));
                    break;
                }
            }
        }
    }

    fn wait_for(&mut self, id: u64) -> Result<Option<DirectoryBatch<R, E>>, WalkError> {
        if let Some(batch) = self.ready.remove(&id) {
            return Ok(Some(batch));
        }
        if let Some(error) = self.schedule_error.take() {
            self.cancel_and_drain();
            return Err(error);
        }
        loop {
            let Ok(result) = self.result_receiver.recv() else {
                if let Some(error) = self.schedule_error.take() {
                    return Err(error);
                }
                return Ok(None);
            };
            self.outstanding = self.outstanding.saturating_sub(1);
            match result.outcome {
                Ok(batch) if result.id == id => {
                    self.refill();
                    if let Some(error) = self.schedule_error.take() {
                        self.cancel_and_drain();
                        return Err(error);
                    }
                    return Ok(Some(batch));
                }
                Ok(batch) => {
                    self.ready.insert(result.id, batch);
                    self.refill();
                }
                Err(payload) => {
                    self.cancel_and_drain();
                    std::panic::resume_unwind(payload);
                }
            }
            if self.cancellation.is_cancelled() {
                self.cancel_and_drain();
                if let Some(error) = self.schedule_error.take() {
                    return Err(error);
                }
                return Ok(None);
            }
        }
    }

    fn prepare_frame(&mut self, batch: DirectoryBatch<R, E>) -> DirectoryFrame<E> {
        let mut items = Vec::with_capacity(batch.entries.len());
        let mut children = Vec::new();
        for item in batch.entries {
            let child = item.as_ref().ok().and_then(|entry| {
                entry.read_children.then(|| {
                    let id = self.next_id;
                    self.next_id = self.next_id.saturating_add(1);
                    let identity = entry.entry.directory_identity();
                    let ancestors = identity.map_or_else(
                        || Arc::clone(&batch.ancestors),
                        |identity| {
                            let mut child = batch.ancestors.as_ref().clone();
                            child.insert(identity);
                            Arc::new(child)
                        },
                    );
                    children.push(DirectoryTask {
                        id,
                        path: entry.path().to_path_buf(),
                        depth: entry.depth(),
                        identity,
                        ancestors,
                        read_state: batch.child_state.clone(),
                    });
                    id
                })
            });
            items.push(PreparedItem { item, child });
        }
        for child in children.into_iter().rev() {
            self.queued.push_front(child);
        }
        self.refill();
        DirectoryFrame {
            items: items.into_iter(),
        }
    }

    fn cancel_and_drain(&mut self) {
        self.cancellation.cancel();
        self.queued.clear();
        while self.outstanding > 0 {
            if self.result_receiver.recv().is_err() {
                break;
            }
            self.outstanding -= 1;
        }
    }
}

fn read_directory<R, E>(
    root: &Arc<PathBuf>,
    root_file_system: Option<FileSystemId>,
    options: WalkOptions,
    cancellation: &CancellationToken,
    processor: Option<&DirectoryProcessor<R, E>>,
    mut task: DirectoryTask<R>,
) -> DirectoryBatch<R, E>
where
    R: Clone + Send + 'static,
    E: Default + Send + 'static,
{
    let ancestors = Arc::clone(&task.ancestors);
    let mut worker_options = options;
    worker_options.error_policy = ErrorPolicy::Continue;
    worker_options.min_depth = 0;
    worker_options.max_open = 1;
    worker_options.max_depth = Some(
        options
            .max_depth
            .unwrap_or(task.depth.saturating_add(1))
            .min(task.depth.saturating_add(1)),
    );
    let mut walker = Walker::from_known_directory_with_ancestry(
        root,
        task.path.clone(),
        task.depth,
        worker_options,
        root_file_system,
        task.identity,
        task.ancestors.as_ref().clone(),
    );
    let mut entries = Vec::new();
    while !cancellation.is_cancelled() {
        let Some(item) = walker.next() else {
            break;
        };
        match item {
            Ok(mut entry) => {
                if entry.is_dir()
                    && entry.skip_reason() == Some(WalkSkipReason::MaxDepth)
                    && options
                        .max_depth
                        .is_none_or(|maximum| entry.depth() < maximum)
                {
                    entry.clear_depth_skip();
                }
                if entry.is_dir() {
                    walker.skip_current_dir();
                }
                entries.push(Ok(StatefulWalkEntry {
                    read_children: entry.is_dir() && entry.skip_reason().is_none(),
                    entry,
                    state: E::default(),
                }));
            }
            Err(error) => entries.push(Err(error)),
        }
    }
    if let Some(processor) = processor {
        processor(task.depth, &task.path, &mut task.read_state, &mut entries);
    }
    DirectoryBatch {
        entries,
        child_state: task.read_state,
        ancestors,
    }
}

fn requested_workers(runtime: &ParallelRuntime, parallelism: usize, max_open: usize) -> usize {
    let available = runtime.parallelism();
    let requested = if parallelism == 0 {
        available.min(if cfg!(windows) { 16 } else { 8 })
    } else {
        parallelism.min(available)
    };
    requested.min(max_open.max(1)).max(1)
}
