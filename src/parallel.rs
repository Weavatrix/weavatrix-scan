use crate::pool::ThreadPool;
use crate::walker::{ErrorPolicy, WalkEntry, WalkError, WalkOptions, WalkSkipReason, Walker};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::{Arc, mpsc};

/// Collected output from a parallel filesystem walk.
#[derive(Debug)]
pub struct ParallelWalkReport {
    pub entries: Vec<WalkEntry>,
    pub errors: Vec<WalkError>,
}

/// A breadth-oriented parallel walker for wide repository trees.
///
/// Top-level directories are distributed across bounded serial walkers. This
/// keeps the serial `Walker` API streaming and gives broad trees an explicit
/// parallel mode without making scanner output order nondeterministic.
/// Parallel report order is intentionally unspecified; callers that need a
/// stable manifest must sort, as `Scanner` does. Link-following falls back to
/// the serial walker so cycle detection has one authoritative seen set.
pub struct ParallelWalker {
    root: PathBuf,
    options: WalkOptions,
    parallelism: usize,
}

impl ParallelWalker {
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            options: WalkOptions::default(),
            parallelism: 0,
        }
    }

    #[must_use]
    pub const fn options(mut self, options: WalkOptions) -> Self {
        self.options = options;
        self
    }

    /// Sets worker count. Zero selects available parallelism.
    #[must_use]
    pub const fn with_parallelism(mut self, parallelism: usize) -> Self {
        self.parallelism = parallelism;
        self
    }

    /// Walks the tree using bounded top-level parallelism.
    ///
    /// # Errors
    ///
    /// With `ErrorPolicy::Abort`, returns the first local traversal error.
    /// With `ErrorPolicy::Continue`, local errors are collected in the report.
    ///
    /// # Panics
    ///
    /// Panics if an internal worker panics or shared traversal state is poisoned.
    pub fn walk(self) -> Result<ParallelWalkReport, WalkError> {
        if self.options.follow_links {
            return collect_serial(&self.root, self.options);
        }
        let shallow = collect_shallow(&self.root, self.options)?;
        let mut entries = shallow.entries;
        let mut errors = shallow.errors;
        let tasks = shallow.tasks;
        let parallel_root = shallow.root;
        if self.options.error_policy == ErrorPolicy::Abort && !errors.is_empty() {
            return Err(errors.remove(0));
        }
        if tasks.is_empty() {
            return Ok(ParallelWalkReport { entries, errors });
        }

        let pool = ThreadPool::global();
        let available = pool.workers();
        let requested = if self.parallelism == 0 {
            available
        } else {
            self.parallelism
        };
        let worker_count = requested
            .min(self.options.max_open.max(1))
            .min(tasks.len())
            .max(1);
        let mut lanes = (0..worker_count)
            .map(|_| Vec::new())
            .collect::<Vec<Vec<DirectoryTask>>>();
        for (index, task) in tasks.into_iter().enumerate() {
            lanes[index % worker_count].push(task);
        }
        let (sender, receiver) = mpsc::channel();
        for lane in lanes {
            let sender = sender.clone();
            let root = Arc::clone(&parallel_root);
            pool.execute(move || {
                let report = collect_lane(lane, self.options, &root);
                let _ = sender.send(report);
            });
        }
        drop(sender);
        for mut report in receiver {
            entries.append(&mut report.entries);
            errors.append(&mut report.errors);
        }
        if self.options.error_policy == ErrorPolicy::Abort && !errors.is_empty() {
            return Err(errors.remove(0));
        }
        Ok(ParallelWalkReport { entries, errors })
    }
}

fn collect_lane(
    tasks: Vec<DirectoryTask>,
    options: WalkOptions,
    parallel_root: &Arc<PathBuf>,
) -> ParallelWalkReport {
    let mut report = ParallelWalkReport {
        entries: Vec::new(),
        errors: Vec::new(),
    };
    for task in tasks {
        let mut worker_options = options;
        worker_options.error_policy = ErrorPolicy::Continue;
        worker_options.max_open = 1;
        worker_options.max_depth = if options.same_file_system {
            options
                .max_depth
                .map(|depth| depth.saturating_sub(task.depth))
        } else {
            options.max_depth
        };
        if options.same_file_system {
            let walker = match Walker::with_options(&task.path, worker_options) {
                Ok(walker) => walker,
                Err(mut error) => {
                    error.rebase_depth(task.depth);
                    report.errors.push(error);
                    continue;
                }
            };
            collect_rebased(walker, parallel_root, task.depth, &mut report);
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
    }
    report
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

struct DirectoryTask {
    path: PathBuf,
    depth: usize,
}

struct ShallowWalk {
    root: Arc<PathBuf>,
    entries: Vec<WalkEntry>,
    errors: Vec<WalkError>,
    tasks: VecDeque<DirectoryTask>,
}

fn collect_shallow(root: &Path, options: WalkOptions) -> Result<ShallowWalk, WalkError> {
    let mut shallow_options = options;
    shallow_options.error_policy = ErrorPolicy::Continue;
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
                entries.push(entry);
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

fn collect_serial(root: &Path, options: WalkOptions) -> Result<ParallelWalkReport, WalkError> {
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
