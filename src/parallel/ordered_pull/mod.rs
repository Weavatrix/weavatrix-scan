use super::ParallelWalker;
use super::pull::{ParallelWalkIter, PullBatch};
use crate::control::CancellationToken;
use crate::runtime::ParallelRuntime;
use crate::walk_types::{DirectoryIdentity, FileSystemId};
use crate::walker::{
    ErrorPolicy, WalkEntry, WalkError, WalkOperation, WalkOptions, WalkSkipReason, Walker,
};
use std::any::Any;
use std::collections::{HashMap, HashSet, VecDeque};
use std::io;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{self, SyncSender, sync_channel};

mod execution;
mod scheduler;

use execution::{ordered_parallel, ordered_serial};

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
    runtime: ParallelRuntime,
    limit: usize,
    next_id: u64,
    queued: VecDeque<DirectoryTask>,
    outstanding: usize,
    ready: HashMap<u64, DirectoryBatch>,
    result_sender: mpsc::Sender<WorkerResult>,
    result_receiver: mpsc::Receiver<WorkerResult>,
    schedule_error: Option<WalkError>,
}

impl ParallelWalker {
    /// Starts bounded parallel traversal and yields entries in strict,
    /// deterministic depth-first order.
    ///
    /// Directory reads are prefetched up to `max_open` and the configured
    /// parallelism. A capacity of zero is normalized to one.
    ///
    /// # Panics
    ///
    /// Panics if the coordinator thread cannot be created. Use
    /// [`Self::try_into_iter_ordered_bounded`] for fallible startup.
    #[must_use]
    pub fn into_iter_ordered_bounded(self, capacity: usize) -> ParallelWalkIter {
        self.try_into_iter_ordered_bounded(capacity)
            .expect("ordered parallel pull coordinator thread can be created")
    }

    /// Fallible form of [`Self::into_iter_ordered_bounded`].
    ///
    /// # Errors
    ///
    /// Returns the coordinator thread spawn error.
    pub fn try_into_iter_ordered_bounded(self, capacity: usize) -> io::Result<ParallelWalkIter> {
        let capacity = capacity.max(1);
        let (sender, receiver) = sync_channel(capacity.saturating_sub(1));
        let cancellation = CancellationToken::new();
        let coordinator_cancellation = cancellation.clone();
        let use_serial = self.runtime.is_worker_thread();
        let coordinator = std::thread::Builder::new()
            .name("weavatrix-scan-ordered-pull".to_owned())
            .spawn(move || {
                if use_serial {
                    ordered_serial(&self, &coordinator_cancellation, &sender);
                } else {
                    ordered_parallel(&self, &coordinator_cancellation, &sender);
                }
            })?;
        Ok(ParallelWalkIter::from_coordinator(
            receiver,
            cancellation,
            coordinator,
        ))
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
