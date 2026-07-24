use super::{BatchControl, DirectoryTask, Shared, TaskReport};
use crate::control::CancellationToken;
use crate::pool::ThreadPool;
use crate::walk_platform::FileSystemId;
use crate::walker::{ErrorPolicy, WalkEntry, WalkError, WalkOptions, Walker};
use std::path::PathBuf;
use std::sync::Arc;

pub(super) fn worker<F>(
    shared: &Shared,
    root: &Arc<PathBuf>,
    root_file_system: Option<FileSystemId>,
    options: WalkOptions,
    cancellation: &CancellationToken,
    visitor: &F,
) where
    F: Fn(&[WalkEntry], &[WalkError]) -> BatchControl + Sync,
{
    while let Some(task) = next_task(shared, cancellation) {
        let report = visit_directory(task, root, root_file_system, options, cancellation, visitor);
        finish_task(shared, report, options.error_policy, cancellation);
    }
}

pub(super) fn stream_worker<F>(
    shared: &Shared,
    root: &Arc<PathBuf>,
    root_file_system: Option<FileSystemId>,
    options: WalkOptions,
    cancellation: &CancellationToken,
    visitor: &F,
) where
    F: Fn(Vec<WalkEntry>, &[WalkError]) -> bool + Sync,
{
    while let Some(task) = next_task(shared, cancellation) {
        let report = stream_directory(task, root, root_file_system, options, cancellation, visitor);
        finish_task(shared, report, options.error_policy, cancellation);
    }
}

fn next_task(shared: &Shared, cancellation: &CancellationToken) -> Option<DirectoryTask> {
    let mut state = shared
        .state
        .lock()
        .expect("dynamic traversal state is not poisoned");
    loop {
        if cancellation.is_cancelled() || state.stopped {
            return None;
        }
        if let Some(task) = state.queue.pop_front() {
            state.active += 1;
            return Some(task);
        }
        if state.active == 0 {
            state.stopped = true;
            shared.ready.notify_all();
            return None;
        }
        state = shared
            .ready
            .wait(state)
            .expect("dynamic traversal state is not poisoned");
    }
}

fn visit_directory<F>(
    task: DirectoryTask,
    root: &Arc<PathBuf>,
    root_file_system: Option<FileSystemId>,
    options: WalkOptions,
    cancellation: &CancellationToken,
    visitor: &F,
) -> TaskReport
where
    F: Fn(&[WalkEntry], &[WalkError]) -> BatchControl + Sync,
{
    let mut worker_options = options;
    worker_options.error_policy = ErrorPolicy::Continue;
    worker_options.min_depth = 0;
    worker_options.max_open = 1;
    let mut walker = Walker::from_known_directory(
        root,
        task.path,
        task.depth,
        worker_options,
        root_file_system,
    );
    let mut entries = Vec::new();
    let mut errors = Vec::new();
    while !cancellation.is_cancelled() {
        let Some(item) = walker.next() else {
            break;
        };
        match item {
            Ok(entry) => {
                if entry.is_dir() {
                    walker.skip_current_dir();
                }
                entries.push(entry);
            }
            Err(error) => errors.push(error),
        }
    }
    report_for(&entries, errors, task.depth, options, visitor)
}

fn stream_directory<F>(
    task: DirectoryTask,
    root: &Arc<PathBuf>,
    root_file_system: Option<FileSystemId>,
    options: WalkOptions,
    cancellation: &CancellationToken,
    visitor: &F,
) -> TaskReport
where
    F: Fn(Vec<WalkEntry>, &[WalkError]) -> bool + Sync,
{
    let mut worker_options = options;
    worker_options.error_policy = ErrorPolicy::Continue;
    worker_options.min_depth = 0;
    worker_options.max_open = 1;
    let mut walker = Walker::from_known_directory(
        root,
        task.path,
        task.depth,
        worker_options,
        root_file_system,
    );
    let mut entries = Vec::new();
    let mut errors = Vec::new();
    while !cancellation.is_cancelled() {
        let Some(item) = walker.next() else {
            break;
        };
        match item {
            Ok(entry) => {
                if entry.is_dir() {
                    walker.skip_current_dir();
                }
                entries.push(entry);
            }
            Err(error) => errors.push(error),
        }
    }
    let visible = task.depth.saturating_add(1) >= options.min_depth;
    let visited = if visible {
        u64::try_from(entries.len()).unwrap_or(u64::MAX)
    } else {
        0
    };
    let directories = entries
        .iter()
        .filter(|entry| entry.is_dir() && entry.skip_reason().is_none())
        .map(|entry| DirectoryTask {
            path: entry.path().to_path_buf(),
            depth: entry.depth(),
        })
        .collect();
    let keep_going = if visible || !errors.is_empty() {
        visitor(if visible { entries } else { Vec::new() }, &errors)
    } else {
        true
    };
    TaskReport {
        directories,
        errors,
        visited,
        quit: !keep_going,
    }
}

fn report_for<F>(
    entries: &[WalkEntry],
    errors: Vec<WalkError>,
    parent_depth: usize,
    options: WalkOptions,
    visitor: &F,
) -> TaskReport
where
    F: Fn(&[WalkEntry], &[WalkError]) -> BatchControl + Sync,
{
    let visible = parent_depth.saturating_add(1) >= options.min_depth;
    let mut decision = if visible || !errors.is_empty() {
        visitor(if visible { entries } else { &[] }, &errors)
    } else {
        BatchControl::continue_all(entries.len())
    };
    if visible {
        decision
            .entries
            .resize(entries.len(), super::WalkControl::Continue);
    } else {
        decision.entries = vec![super::WalkControl::Continue; entries.len()];
    }
    let directories = entries
        .iter()
        .zip(&decision.entries)
        .filter(|(entry, control)| {
            entry.is_dir()
                && entry.skip_reason().is_none()
                && **control == super::WalkControl::Continue
                && !decision.quit
        })
        .map(|(entry, _)| DirectoryTask {
            path: entry.path().to_path_buf(),
            depth: entry.depth(),
        })
        .collect();
    let abort = options.error_policy == ErrorPolicy::Abort && !errors.is_empty();
    TaskReport {
        directories,
        errors,
        visited: if visible {
            u64::try_from(entries.len()).unwrap_or(u64::MAX)
        } else {
            0
        },
        quit: decision.quit || abort,
    }
}

fn finish_task(
    shared: &Shared,
    report: TaskReport,
    error_policy: ErrorPolicy,
    cancellation: &CancellationToken,
) {
    let mut state = shared
        .state
        .lock()
        .expect("dynamic traversal state is not poisoned");
    state.active -= 1;
    state.visited = state.visited.saturating_add(report.visited);
    let abort = error_policy == ErrorPolicy::Abort && !report.errors.is_empty();
    state.errors.extend(report.errors);
    state.quit |= report.quit;
    state.stopped |= report.quit || abort || cancellation.is_cancelled();
    if !state.stopped {
        state.queue.extend(report.directories);
    }
    if state.active == 0 && state.queue.is_empty() {
        state.stopped = true;
    }
    shared.ready.notify_all();
}

pub(super) fn worker_count(parallelism: usize, max_open: usize) -> usize {
    let available = ThreadPool::global().workers();
    let requested = if parallelism == 0 {
        available.min(if cfg!(windows) { 4 } else { 8 })
    } else {
        parallelism
    };
    requested.min(available).min(max_open.max(1)).max(1)
}
