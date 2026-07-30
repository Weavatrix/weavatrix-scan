use super::{DirectoryProcessor, StatefulWalkBuilder, StatefulWalkEntry};
use crate::control::CancellationToken;
use crate::runtime::ParallelRuntime;
use crate::walk_types::{
    DirectoryIdentity, ErrorPolicy, FileSystemId, WalkError, WalkOperation, WalkOptions,
    WalkSkipReason,
};
use crate::walker::Walker;
use std::any::Any;
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, SyncSender, sync_channel};
use std::thread::JoinHandle;

mod execution;
mod scheduler;

use execution::{run_parallel, run_serial};

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
