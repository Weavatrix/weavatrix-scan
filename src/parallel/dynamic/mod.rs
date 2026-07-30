use super::visit::{ParallelVisitReport, WalkControl, WalkEvent};
use crate::control::CancellationToken;
use crate::report::FileIdentity;
use crate::runtime::ParallelRuntime;
use crate::walk_types::{DirectoryIdentity, FileSystemId};
use crate::walker::{ErrorPolicy, WalkEntry, WalkError, WalkOperation, WalkOptions, Walker};
use std::collections::{HashSet, VecDeque};
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, mpsc};

mod api;
mod serial;
mod worker;

pub(super) use api::visit;
pub(crate) use api::{stream_batched, visit_batched};
use serial::{stream_batched_serial, visit_batched_serial};

use worker::{stream_worker, worker, worker_count};

struct DirectoryTask {
    path: PathBuf,
    depth: usize,
    identity: Option<DirectoryIdentity>,
    ancestors: Arc<HashSet<DirectoryIdentity>>,
}

struct SharedState {
    queue: VecDeque<DirectoryTask>,
    active: usize,
    stopped: bool,
    quit: bool,
    visited: u64,
    errors: Vec<WalkError>,
}

struct Shared {
    state: Mutex<SharedState>,
    ready: Condvar,
}

struct TaskReport {
    directories: Vec<DirectoryTask>,
    errors: Vec<WalkError>,
    visited: u64,
    quit: bool,
}

pub(crate) struct BatchControl {
    pub(crate) entries: Vec<WalkControl>,
    pub(crate) quit: bool,
}

impl BatchControl {
    pub(crate) fn continue_all(entries: usize) -> Self {
        Self {
            entries: vec![WalkControl::Continue; entries],
            quit: false,
        }
    }
}

fn run_batched<F>(
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
    let (parallel_root, root_file_system, root_entry) = prepare_root(root, options)?;
    let root_identity = root_entry.directory_identity();
    let mut visited = 0_u64;
    let mut root_control = WalkControl::Continue;
    if !cancellation.is_cancelled() && root_entry.depth() >= options.min_depth {
        visited = 1;
        let decision = visitor(std::slice::from_ref(&root_entry), &[]);
        root_control = decision
            .entries
            .first()
            .copied()
            .unwrap_or(WalkControl::Continue);
        if decision.quit {
            root_control = WalkControl::Quit;
        }
    }
    let descend = !cancellation.is_cancelled()
        && root_entry.skip_reason().is_none()
        && root_control == WalkControl::Continue;
    if !descend {
        return Ok(ParallelVisitReport {
            visited,
            errors: Vec::new(),
            quit: root_control == WalkControl::Quit,
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
                worker(
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

    finish_report(shared, &cancellation, options.error_policy)
}

fn finish_report(
    shared: Arc<Shared>,
    cancellation: &CancellationToken,
    error_policy: ErrorPolicy,
) -> Result<ParallelVisitReport, WalkError> {
    let shared = Arc::try_unwrap(shared)
        .ok()
        .expect("dynamic traversal workers released shared state");
    let mut state = shared
        .state
        .into_inner()
        .expect("dynamic traversal state is not poisoned");
    if error_policy == ErrorPolicy::Abort && !state.errors.is_empty() {
        return Err(state.errors.remove(0));
    }
    Ok(ParallelVisitReport {
        visited: state.visited,
        errors: state.errors,
        quit: state.quit,
        cancelled: cancellation.is_cancelled(),
    })
}

fn schedule_error(root: &Path, source: std::io::Error) -> WalkError {
    WalkError::new(root, 0, WalkOperation::ScheduleWorker, source)
}

fn prepare_root(
    root: &Path,
    options: WalkOptions,
) -> Result<(Arc<PathBuf>, Option<FileSystemId>, WalkEntry), WalkError> {
    let mut root_options = options;
    root_options.min_depth = 0;
    let mut walker = Walker::with_options(root, root_options)?;
    let root_file_system = walker.root_file_system;
    let parallel_root = Arc::clone(&walker.root);
    let entry = walker.next().expect("a validated root yields one entry")?;
    Ok((parallel_root, root_file_system, entry))
}

fn initial_state(
    root: &Arc<PathBuf>,
    visited: u64,
    root_identity: Option<DirectoryIdentity>,
) -> Arc<Shared> {
    let mut ancestors = HashSet::new();
    if let Some(identity) = root_identity {
        ancestors.insert(identity);
    }
    Arc::new(Shared {
        state: Mutex::new(SharedState {
            queue: VecDeque::from([DirectoryTask {
                path: root.as_ref().clone(),
                depth: 0,
                identity: root_identity,
                ancestors: Arc::new(ancestors),
            }]),
            active: 0,
            stopped: false,
            quit: false,
            visited,
            errors: Vec::new(),
        }),
        ready: Condvar::new(),
    })
}
