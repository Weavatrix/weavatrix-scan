use super::ParallelWalker;
use super::pull::{ParallelWalkIter, PullBatch};
use crate::control::CancellationToken;
use crate::pool::ThreadPool;
use crate::walk_platform::{DirectoryIdentity, FileSystemId};
use crate::walker::{ErrorPolicy, WalkEntry, WalkError, WalkOptions, WalkSkipReason, Walker};
use std::any::Any;
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{self, SyncSender, sync_channel};

struct DirectoryTask {
    id: u64,
    path: PathBuf,
    depth: usize,
    identity: Option<DirectoryIdentity>,
    ancestors: Arc<HashSet<DirectoryIdentity>>,
}

struct WorkerResult {
    id: u64,
    outcome: Result<DirectoryBatch, Box<dyn Any + Send>>,
}

struct DirectoryBatch {
    entries: Vec<Result<WalkEntry, WalkError>>,
    ancestors: Arc<HashSet<DirectoryIdentity>>,
}

struct PreparedItem {
    item: Result<WalkEntry, WalkError>,
    child: Option<u64>,
}

struct DirectoryFrame {
    items: std::vec::IntoIter<PreparedItem>,
}

struct OrderedScheduler {
    root: Arc<PathBuf>,
    root_file_system: Option<FileSystemId>,
    options: WalkOptions,
    cancellation: CancellationToken,
    limit: usize,
    next_id: u64,
    queued: VecDeque<DirectoryTask>,
    outstanding: usize,
    ready: HashMap<u64, DirectoryBatch>,
    result_sender: mpsc::Sender<WorkerResult>,
    result_receiver: mpsc::Receiver<WorkerResult>,
}

impl ParallelWalker {
    /// Starts bounded parallel traversal and yields entries in strict,
    /// deterministic depth-first order.
    ///
    /// Directory reads are prefetched up to `max_open` and the configured
    /// parallelism. A capacity of zero is normalized to one.
    #[must_use]
    pub fn into_iter_ordered_bounded(self, capacity: usize) -> ParallelWalkIter {
        let capacity = capacity.max(1);
        let (sender, receiver) = sync_channel(capacity.saturating_sub(1));
        let cancellation = CancellationToken::new();
        let coordinator_cancellation = cancellation.clone();
        let use_serial = ThreadPool::is_worker_thread();
        let coordinator = std::thread::spawn(move || {
            if use_serial {
                ordered_serial(&self, &coordinator_cancellation, &sender);
            } else {
                ordered_parallel(&self, &coordinator_cancellation, &sender);
            }
        });
        ParallelWalkIter::from_coordinator(receiver, cancellation, coordinator)
    }
}

fn ordered_serial(
    walker: &ParallelWalker,
    cancellation: &CancellationToken,
    sender: &SyncSender<PullBatch>,
) {
    let error_policy = walker.options.error_policy;
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
        if sender.send(vec![item]).is_err() || abort {
            break;
        }
    }
}

fn ordered_parallel(
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
    if root_entry.depth() >= options.min_depth && sender.send(vec![Ok(root_entry)]).is_err() {
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
        limit: super::requested_workers(walker.parallelism, options.max_open),
        next_id: 1,
        queued: VecDeque::new(),
        outstanding: 0,
        ready: HashMap::new(),
        result_sender,
        result_receiver,
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
    let Some(root_batch) = scheduler.wait_for(0) else {
        return;
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
        if visible && sender.send(vec![prepared.item]).is_err() {
            scheduler.cancel_and_drain();
            return;
        }
        if abort {
            scheduler.cancel_and_drain();
            return;
        }
        if let Some(child) = child {
            let Some(batch) = scheduler.wait_for(child) else {
                return;
            };
            frames.push(scheduler.prepare_frame(batch));
        }
    }
    scheduler.cancel_and_drain();
}

impl OrderedScheduler {
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
            self.outstanding += 1;
            ThreadPool::global().execute(move || {
                let id = task.id;
                let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    read_directory(&root, root_file_system, options, &cancellation, task)
                }));
                let _ = result_sender.send(WorkerResult { id, outcome });
            });
        }
    }

    fn wait_for(&mut self, id: u64) -> Option<DirectoryBatch> {
        if let Some(batch) = self.ready.remove(&id) {
            return Some(batch);
        }
        loop {
            let result = self
                .result_receiver
                .recv()
                .expect("every ordered directory worker reports completion");
            self.outstanding = self.outstanding.saturating_sub(1);
            match result.outcome {
                Ok(batch) if result.id == id => {
                    self.refill();
                    return Some(batch);
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
                return None;
            }
        }
    }

    fn prepare_frame(&mut self, mut batch: DirectoryBatch) -> DirectoryFrame {
        batch.entries.sort_by(|left, right| {
            let left_path = left
                .as_ref()
                .map_or_else(|error| error.path(), WalkEntry::path);
            let right_path = right
                .as_ref()
                .map_or_else(|error| error.path(), WalkEntry::path);
            left_path.cmp(right_path)
        });
        let mut items = Vec::with_capacity(batch.entries.len());
        let mut children = Vec::new();
        for item in batch.entries {
            let child = item.as_ref().ok().and_then(|entry| {
                (entry.is_dir() && entry.skip_reason().is_none()).then(|| {
                    let id = self.next_id;
                    self.next_id = self.next_id.saturating_add(1);
                    let identity = entry.directory_identity();
                    let mut ancestors = batch.ancestors.as_ref().clone();
                    if let Some(identity) = identity {
                        ancestors.insert(identity);
                    }
                    children.push(DirectoryTask {
                        id,
                        path: entry.path().to_path_buf(),
                        depth: entry.depth(),
                        identity,
                        ancestors: Arc::new(ancestors),
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

fn read_directory(
    root: &Arc<PathBuf>,
    root_file_system: Option<FileSystemId>,
    options: WalkOptions,
    cancellation: &CancellationToken,
    task: DirectoryTask,
) -> DirectoryBatch {
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
        task.path,
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
                entries.push(Ok(entry));
            }
            Err(error) => entries.push(Err(error)),
        }
    }
    DirectoryBatch { entries, ancestors }
}
