use crate::pool::ThreadPool;
use crate::walker::{ErrorPolicy, WalkEntry, WalkError, WalkOptions};
use std::path::PathBuf;
use std::sync::{Arc, mpsc};

mod collect;
pub(crate) mod dynamic;
mod visit;
mod visit_worker;

use collect::{DirectoryTask, collect_lane, collect_serial, collect_shallow, expand_frontier};
pub use visit::{ParallelVisitReport, WalkControl, WalkEvent};

/// Collected output from a parallel filesystem walk.
#[derive(Debug)]
pub struct ParallelWalkReport {
    pub entries: Vec<WalkEntry>,
    pub errors: Vec<WalkError>,
}

/// An adaptive parallel filesystem walker.
///
/// Broad root frontiers use low-overhead lane traversal. Narrow trees use
/// dynamic directory scheduling so work below one top-level directory can use
/// every worker. Link-following falls back to the serial walker so cycle
/// detection has one authoritative seen set.
pub struct ParallelWalker {
    pub(super) root: PathBuf,
    pub(super) options: WalkOptions,
    pub(super) parallelism: usize,
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

    /// Walks the tree using bounded adaptive scheduling.
    ///
    /// # Errors
    ///
    /// With `ErrorPolicy::Abort`, returns the first observed local traversal
    /// error. With `ErrorPolicy::Continue`, local errors are collected.
    ///
    /// # Panics
    ///
    /// Panics if an internal worker panics or shared traversal state is
    /// poisoned.
    pub fn walk(mut self) -> Result<ParallelWalkReport, WalkError> {
        self.options = self.options.normalized();
        if self.options.follow_links {
            return collect_serial(&self.root, self.options);
        }
        let mut shallow = collect_shallow(&self.root, self.options)?;
        if self.options.error_policy == ErrorPolicy::Abort && !shallow.errors.is_empty() {
            return Err(shallow.errors.into_iter().next().expect("error exists"));
        }
        if !self.options.same_file_system && shallow.tasks.len() < 2 {
            let target = requested_workers(self.parallelism, self.options.max_open);
            shallow = expand_frontier(shallow, self.options, target);
        }
        if shallow.tasks.is_empty() {
            return Ok(ParallelWalkReport {
                entries: shallow.entries,
                errors: shallow.errors,
            });
        }
        self.walk_lanes(shallow)
    }

    fn walk_lanes(self, shallow: collect::ShallowWalk) -> Result<ParallelWalkReport, WalkError> {
        let mut entries = shallow.entries;
        let mut errors = shallow.errors;
        let task_count = shallow.tasks.len();
        let worker_count =
            parallel_worker_count(self.parallelism, self.options.max_open, task_count);
        let mut lanes = (0..worker_count)
            .map(|_| Vec::<DirectoryTask>::new())
            .collect::<Vec<_>>();
        for (index, task) in shallow.tasks.into_iter().enumerate() {
            lanes[index % worker_count].push(task);
        }
        let (sender, receiver) = mpsc::channel();
        for (index, lane) in lanes.into_iter().enumerate() {
            let sender = sender.clone();
            let root = Arc::clone(&shallow.root);
            let options = self.options;
            ThreadPool::global().execute(move || {
                let report = collect_lane(lane, options, &root);
                let _ = sender.send((index, report));
            });
        }
        drop(sender);
        let mut completed = (0..worker_count).map(|_| None).collect::<Vec<_>>();
        for (index, report) in receiver {
            completed[index] = Some(report);
        }
        let mut lanes = completed
            .into_iter()
            .map(|lane| {
                let lane = lane.expect("every parallel lane reports completion");
                (
                    lane.report.entries.into_iter(),
                    lane.report.errors.into_iter(),
                    lane.segments,
                )
            })
            .collect::<Vec<_>>();
        for task_index in 0..task_count {
            let (lane_entries, lane_errors, segments) = &mut lanes[task_index % worker_count];
            let segment = segments
                .pop_front()
                .expect("every directory task has an output segment");
            entries.extend(lane_entries.by_ref().take(segment.entries));
            errors.extend(lane_errors.by_ref().take(segment.errors));
        }
        if self.options.error_policy == ErrorPolicy::Abort && !errors.is_empty() {
            return Err(errors.remove(0));
        }
        Ok(ParallelWalkReport { entries, errors })
    }
}

fn parallel_worker_count(parallelism: usize, max_open: usize, tasks: usize) -> usize {
    requested_workers(parallelism, max_open).min(tasks).max(1)
}

fn requested_workers(parallelism: usize, max_open: usize) -> usize {
    let available = ThreadPool::global().workers();
    if parallelism == 0 {
        available.min(default_traversal_workers())
    } else {
        parallelism.min(available)
    }
    .min(max_open.max(1))
    .max(1)
}

const fn default_traversal_workers() -> usize {
    if cfg!(windows) { 4 } else { 8 }
}
