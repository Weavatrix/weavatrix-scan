use super::collect::DirectoryTask;
use super::visit::{ParallelVisitReport, WalkControl, WalkEvent};
use crate::control::CancellationToken;
use crate::walker::{ErrorPolicy, WalkError, WalkOptions, Walker};
use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

pub(super) struct VisitLaneReport {
    pub(super) visited: u64,
    pub(super) errors: Vec<WalkError>,
}

pub(super) fn visit_lane<F>(
    tasks: Vec<DirectoryTask>,
    options: WalkOptions,
    parallel_root: &Arc<PathBuf>,
    cancellation: &CancellationToken,
    stop: &AtomicBool,
    visitor: &F,
) -> VisitLaneReport
where
    F: for<'entry> Fn(WalkEvent<'entry>) -> WalkControl + Sync,
{
    let mut report = VisitLaneReport {
        visited: 0,
        errors: Vec::new(),
    };
    for task in tasks {
        if cancellation.is_cancelled() || stop.load(Ordering::Acquire) {
            break;
        }
        let mut worker_options = options;
        worker_options.error_policy = ErrorPolicy::Continue;
        worker_options.max_open = 1;
        worker_options.min_depth = if options.same_file_system {
            options.min_depth.saturating_sub(task.depth)
        } else {
            options.min_depth
        };
        worker_options.max_depth = if options.same_file_system {
            options
                .max_depth
                .map(|depth| depth.saturating_sub(task.depth))
        } else {
            options.max_depth
        };
        if options.same_file_system {
            match Walker::with_options(&task.path, worker_options) {
                Ok(walker) => visit_walker(
                    walker,
                    Some((parallel_root, task.depth)),
                    options.error_policy,
                    cancellation,
                    stop,
                    visitor,
                    &mut report,
                ),
                Err(mut error) => {
                    error.rebase_depth(task.depth);
                    visit_error(error, options.error_policy, stop, visitor, &mut report);
                }
            }
        } else {
            let walker =
                Walker::from_known_directory(parallel_root, task.path, task.depth, worker_options);
            visit_walker(
                walker,
                None,
                options.error_policy,
                cancellation,
                stop,
                visitor,
                &mut report,
            );
        }
    }
    report
}

fn visit_walker<F>(
    mut walker: Walker,
    rebase: Option<(&Arc<PathBuf>, usize)>,
    error_policy: ErrorPolicy,
    cancellation: &CancellationToken,
    stop: &AtomicBool,
    visitor: &F,
    report: &mut VisitLaneReport,
) where
    F: for<'entry> Fn(WalkEvent<'entry>) -> WalkControl + Sync,
{
    while !cancellation.is_cancelled() && !stop.load(Ordering::Acquire) {
        let Some(item) = walker.next() else {
            break;
        };
        match item {
            Ok(entry) if rebase.is_some() && entry.depth() == 0 => {}
            Ok(mut entry) => {
                if let Some((root, depth)) = rebase {
                    entry.rebase(root, depth);
                }
                report.visited = report.visited.saturating_add(1);
                match visitor(WalkEvent::Entry(&entry)) {
                    WalkControl::Skip if entry.is_dir() => walker.skip_current_dir(),
                    WalkControl::Continue | WalkControl::Skip => {}
                    WalkControl::Quit => stop.store(true, Ordering::Release),
                }
            }
            Err(mut error) => {
                if let Some((_, depth)) = rebase {
                    error.rebase_depth(depth);
                }
                visit_error(error, error_policy, stop, visitor, report);
            }
        }
    }
}

fn visit_error<F>(
    error: WalkError,
    error_policy: ErrorPolicy,
    stop: &AtomicBool,
    visitor: &F,
    report: &mut VisitLaneReport,
) where
    F: for<'entry> Fn(WalkEvent<'entry>) -> WalkControl + Sync,
{
    let control = visitor(WalkEvent::Error(&error));
    report.errors.push(error);
    if error_policy == ErrorPolicy::Abort || control == WalkControl::Quit {
        stop.store(true, Ordering::Release);
    }
}

pub(super) fn visit_serial<F>(
    root: &Path,
    options: WalkOptions,
    cancellation: &CancellationToken,
    visitor: F,
) -> Result<ParallelVisitReport, WalkError>
where
    F: for<'entry> Fn(WalkEvent<'entry>) -> WalkControl + Sync,
{
    let mut walker_options = options;
    walker_options.error_policy = ErrorPolicy::Continue;
    let walker = Walker::with_options(root, walker_options)?;
    let stop = AtomicBool::new(false);
    let mut report = VisitLaneReport {
        visited: 0,
        errors: Vec::new(),
    };
    visit_walker(
        walker,
        None,
        options.error_policy,
        cancellation,
        &stop,
        &visitor,
        &mut report,
    );
    if options.error_policy == ErrorPolicy::Abort && !report.errors.is_empty() {
        return Err(report.errors.remove(0));
    }
    Ok(ParallelVisitReport {
        visited: report.visited,
        errors: report.errors,
        quit: stop.load(Ordering::Acquire),
        cancelled: cancellation.is_cancelled(),
    })
}
