use super::visit::{ParallelVisitReport, WalkControl, WalkEvent};
use crate::control::CancellationToken;
use crate::pool::ThreadPool;
use crate::walk_platform::FileSystemId;
use crate::walker::{ErrorPolicy, WalkEntry, WalkError, WalkOptions, Walker};
use std::collections::VecDeque;
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, mpsc};

mod worker;

use worker::{stream_worker, worker, worker_count};

struct DirectoryTask {
    path: PathBuf,
    depth: usize,
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

pub(super) fn visit<F>(
    root: &Path,
    options: WalkOptions,
    parallelism: usize,
    cancellation: &CancellationToken,
    visitor: F,
) -> Result<ParallelVisitReport, WalkError>
where
    F: for<'entry> Fn(WalkEvent<'entry>) -> WalkControl + Send + Sync + 'static,
{
    visit_batched(
        root,
        options,
        parallelism,
        cancellation,
        move |entries, errors| {
            let mut controls = Vec::with_capacity(entries.len());
            let mut quit = false;
            for entry in entries {
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
    )
}

pub(crate) fn visit_batched<F>(
    root: &Path,
    options: WalkOptions,
    parallelism: usize,
    cancellation: &CancellationToken,
    visitor: F,
) -> Result<ParallelVisitReport, WalkError>
where
    F: Fn(&[WalkEntry], &[WalkError]) -> BatchControl + Send + Sync + 'static,
{
    if ThreadPool::is_worker_thread() {
        return visit_batched_serial(root, options, cancellation, visitor);
    }
    run_batched(root, options, parallelism, cancellation, visitor)
}

pub(crate) fn stream_batched<F>(
    root: &Path,
    options: WalkOptions,
    parallelism: usize,
    cancellation: &CancellationToken,
    visitor: F,
) -> Result<ParallelVisitReport, WalkError>
where
    F: Fn(Vec<WalkEntry>, &[WalkError]) -> bool + Send + Sync + 'static,
{
    if ThreadPool::is_worker_thread() {
        return stream_batched_serial(root, options, cancellation, visitor);
    }
    let (parallel_root, root_file_system, root_entry) = prepare_root(root, options)?;
    let root_can_descend = root_entry.skip_reason().is_none();
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

    let shared = initial_state(&parallel_root, visited);
    let visitor = Arc::new(visitor);
    let cancellation = cancellation.clone();
    let worker_count = worker_count(parallelism, options.max_open);
    let (sender, receiver) = mpsc::channel();
    for _ in 0..worker_count {
        let shared = Arc::clone(&shared);
        let visitor = Arc::clone(&visitor);
        let root = Arc::clone(&parallel_root);
        let cancellation = cancellation.clone();
        let sender = sender.clone();
        ThreadPool::global().execute(move || {
            let outcome = catch_unwind(AssertUnwindSafe(|| {
                stream_worker(
                    &shared,
                    &root,
                    root_file_system,
                    options,
                    &cancellation,
                    visitor.as_ref(),
                );
            }));
            if outcome.is_err() {
                worker::abort_after_panic(&shared);
            }
            let _ = sender.send(outcome);
        });
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

fn run_batched<F>(
    root: &Path,
    options: WalkOptions,
    parallelism: usize,
    cancellation: &CancellationToken,
    visitor: F,
) -> Result<ParallelVisitReport, WalkError>
where
    F: Fn(&[WalkEntry], &[WalkError]) -> BatchControl + Send + Sync + 'static,
{
    let (parallel_root, root_file_system, root_entry) = prepare_root(root, options)?;
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

    let shared = initial_state(&parallel_root, visited);
    let visitor = Arc::new(visitor);
    let cancellation = cancellation.clone();
    let worker_count = worker_count(parallelism, options.max_open);
    let (sender, receiver) = mpsc::channel();
    for _ in 0..worker_count {
        let shared = Arc::clone(&shared);
        let visitor = Arc::clone(&visitor);
        let root = Arc::clone(&parallel_root);
        let cancellation = cancellation.clone();
        let sender = sender.clone();
        ThreadPool::global().execute(move || {
            let outcome = catch_unwind(AssertUnwindSafe(|| {
                worker(
                    &shared,
                    &root,
                    root_file_system,
                    options,
                    &cancellation,
                    visitor.as_ref(),
                );
            }));
            if outcome.is_err() {
                worker::abort_after_panic(&shared);
            }
            let _ = sender.send(outcome);
        });
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
    Ok(ParallelVisitReport {
        visited: state.visited,
        errors: state.errors,
        quit: state.quit,
        cancelled: cancellation.is_cancelled(),
    })
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

fn initial_state(root: &Arc<PathBuf>, visited: u64) -> Arc<Shared> {
    Arc::new(Shared {
        state: Mutex::new(SharedState {
            queue: VecDeque::from([DirectoryTask {
                path: root.as_ref().clone(),
                depth: 0,
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

fn visit_batched_serial<F>(
    root: &Path,
    mut options: WalkOptions,
    cancellation: &CancellationToken,
    visitor: F,
) -> Result<ParallelVisitReport, WalkError>
where
    F: Fn(&[WalkEntry], &[WalkError]) -> BatchControl,
{
    let error_policy = options.error_policy;
    options.error_policy = ErrorPolicy::Continue;
    let mut walker = Walker::with_options(root, options)?;
    let mut visited = 0_u64;
    let mut errors = Vec::new();
    let mut quit = false;
    while !cancellation.is_cancelled() && !quit {
        let Some(item) = walker.next() else {
            break;
        };
        match item {
            Ok(entry) => {
                visited = visited.saturating_add(1);
                let decision = visitor(std::slice::from_ref(&entry), &[]);
                let control = decision
                    .entries
                    .first()
                    .copied()
                    .unwrap_or(WalkControl::Continue);
                if control == WalkControl::Skip && entry.is_dir() {
                    walker.skip_current_dir();
                }
                quit = decision.quit || control == WalkControl::Quit;
            }
            Err(error) => {
                let decision = visitor(&[], std::slice::from_ref(&error));
                errors.push(error);
                if error_policy == ErrorPolicy::Abort {
                    return Err(errors.remove(0));
                }
                quit = decision.quit;
            }
        }
    }
    Ok(ParallelVisitReport {
        visited,
        errors,
        quit,
        cancelled: cancellation.is_cancelled(),
    })
}

fn stream_batched_serial<F>(
    root: &Path,
    mut options: WalkOptions,
    cancellation: &CancellationToken,
    visitor: F,
) -> Result<ParallelVisitReport, WalkError>
where
    F: Fn(Vec<WalkEntry>, &[WalkError]) -> bool,
{
    let error_policy = options.error_policy;
    options.error_policy = ErrorPolicy::Continue;
    let mut walker = Walker::with_options(root, options)?;
    let mut visited = 0_u64;
    let mut errors = Vec::new();
    let mut quit = false;
    while !cancellation.is_cancelled() && !quit {
        let Some(item) = walker.next() else {
            break;
        };
        match item {
            Ok(entry) => {
                visited = visited.saturating_add(1);
                quit = !visitor(vec![entry], &[]);
            }
            Err(error) => {
                quit = !visitor(Vec::new(), std::slice::from_ref(&error));
                errors.push(error);
                if error_policy == ErrorPolicy::Abort {
                    return Err(errors.remove(0));
                }
            }
        }
    }
    Ok(ParallelVisitReport {
        visited,
        errors,
        quit,
        cancelled: cancellation.is_cancelled(),
    })
}
