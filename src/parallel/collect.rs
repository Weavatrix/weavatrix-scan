use super::ParallelWalkReport;
use crate::walker::{ErrorPolicy, WalkEntry, WalkError, WalkOptions, WalkSkipReason, Walker};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub(super) fn collect_lane(
    tasks: Vec<DirectoryTask>,
    options: WalkOptions,
    parallel_root: &Arc<PathBuf>,
) -> LaneReport {
    let mut report = ParallelWalkReport {
        entries: Vec::new(),
        errors: Vec::new(),
    };
    let mut segments = VecDeque::with_capacity(tasks.len());
    for task in tasks {
        let entry_start = report.entries.len();
        let error_start = report.errors.len();
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
                Ok(walker) => collect_rebased(walker, parallel_root, task.depth, &mut report),
                Err(mut error) => {
                    error.rebase_depth(task.depth);
                    report.errors.push(error);
                }
            }
        } else {
            let walker =
                Walker::from_known_directory(parallel_root, task.path, task.depth, worker_options);
            for item in walker {
                match item {
                    Ok(entry) => report.entries.push(entry),
                    Err(error) => report.errors.push(error),
                }
            }
        }
        segments.push_back(TaskSegment {
            entries: report.entries.len() - entry_start,
            errors: report.errors.len() - error_start,
        });
    }
    LaneReport { report, segments }
}

fn collect_rebased(
    walker: Walker,
    root: &Arc<PathBuf>,
    depth: usize,
    report: &mut ParallelWalkReport,
) {
    for item in walker {
        match item {
            Ok(entry) if entry.depth() == 0 => {}
            Ok(mut entry) => {
                entry.rebase(root, depth);
                report.entries.push(entry);
            }
            Err(mut error) => {
                error.rebase_depth(depth);
                report.errors.push(error);
            }
        }
    }
}

pub(super) struct DirectoryTask {
    pub(super) path: PathBuf,
    pub(super) depth: usize,
}

pub(super) struct TaskSegment {
    pub(super) entries: usize,
    pub(super) errors: usize,
}

pub(super) struct LaneReport {
    pub(super) report: ParallelWalkReport,
    pub(super) segments: VecDeque<TaskSegment>,
}

pub(super) struct ShallowWalk {
    pub(super) root: Arc<PathBuf>,
    pub(super) entries: Vec<WalkEntry>,
    pub(super) errors: Vec<WalkError>,
    pub(super) tasks: VecDeque<DirectoryTask>,
}

pub(super) fn collect_shallow(root: &Path, options: WalkOptions) -> Result<ShallowWalk, WalkError> {
    let mut shallow_options = options;
    shallow_options.error_policy = ErrorPolicy::Continue;
    shallow_options.min_depth = 0;
    shallow_options.max_depth = Some(options.max_depth.unwrap_or(1).min(1));
    let shallow = Walker::with_options(root, shallow_options)?;
    let canonical_root = Arc::new(shallow.root().to_path_buf());
    let mut entries = Vec::new();
    let mut errors = Vec::new();
    let mut tasks = VecDeque::new();

    for item in shallow {
        match item {
            Ok(mut entry) => {
                let can_descend = entry.depth() == 1
                    && entry.is_dir()
                    && entry.skip_reason() == Some(WalkSkipReason::MaxDepth)
                    && options.max_depth.is_none_or(|max_depth| max_depth > 1);
                if can_descend {
                    entry.clear_depth_skip();
                    tasks.push_back(DirectoryTask {
                        path: entry.path().to_path_buf(),
                        depth: entry.depth(),
                    });
                }
                if entry.depth() >= options.min_depth {
                    entries.push(entry);
                }
            }
            Err(error) => errors.push(error),
        }
    }
    Ok(ShallowWalk {
        root: canonical_root,
        entries,
        errors,
        tasks,
    })
}

pub(super) fn collect_serial(
    root: &Path,
    options: WalkOptions,
) -> Result<ParallelWalkReport, WalkError> {
    let mut walker_options = options;
    walker_options.error_policy = ErrorPolicy::Continue;
    let walker = Walker::with_options(root, walker_options)?;
    let mut report = ParallelWalkReport {
        entries: Vec::new(),
        errors: Vec::new(),
    };
    for item in walker {
        match item {
            Ok(entry) => report.entries.push(entry),
            Err(error) => report.errors.push(error),
        }
    }
    if options.error_policy == ErrorPolicy::Abort && !report.errors.is_empty() {
        return Err(report.errors.remove(0));
    }
    Ok(report)
}
