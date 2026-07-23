use crate::pool::ThreadPool;
use crate::walker::{ErrorPolicy, WalkEntry, WalkError, WalkOptions};
use std::path::PathBuf;
use std::sync::{Arc, mpsc};

mod collect;
mod visit;
mod visit_worker;

use collect::{DirectoryTask, collect_lane, collect_serial, collect_shallow};
pub use visit::{ParallelVisitReport, WalkControl, WalkEvent};

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
/// Worker completion order does not affect report order. Callers that need a
/// cross-filesystem stable manifest must still sort, as `Scanner` does.
/// Link-following falls back to the serial walker so cycle detection has one
/// authoritative seen set.
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
    pub fn walk(mut self) -> Result<ParallelWalkReport, WalkError> {
        self.options = self.options.normalized();
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
        let worker_count =
            parallel_worker_count(self.parallelism, self.options.max_open, tasks.len());
        let task_count = tasks.len();
        let mut lanes = (0..worker_count)
            .map(|_| Vec::<DirectoryTask>::new())
            .collect::<Vec<_>>();
        for (index, task) in tasks.into_iter().enumerate() {
            lanes[index % worker_count].push(task);
        }
        let (sender, receiver) = mpsc::channel();
        for (index, lane) in lanes.into_iter().enumerate() {
            let sender = sender.clone();
            let root = Arc::clone(&parallel_root);
            let options = self.options;
            pool.execute(move || {
                let report = collect_lane(lane, options, &root);
                let _ = sender.send((index, report));
            });
        }
        drop(sender);
        let mut completed = (0..worker_count).map(|_| None).collect::<Vec<_>>();
        for (index, report) in receiver {
            completed[index] = Some(report);
        }
        let (additional_entries, additional_errors) =
            completed
                .iter()
                .flatten()
                .fold((0, 0), |(entries, errors), lane| {
                    (
                        entries + lane.report.entries.len(),
                        errors + lane.report.errors.len(),
                    )
                });
        entries.reserve(additional_entries);
        errors.reserve(additional_errors);
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

pub(super) fn parallel_worker_count(parallelism: usize, max_open: usize, tasks: usize) -> usize {
    let available = ThreadPool::global().workers();
    let requested = if parallelism == 0 {
        available
    } else {
        parallelism
    };
    requested.min(max_open.max(1)).min(tasks).max(1)
}
