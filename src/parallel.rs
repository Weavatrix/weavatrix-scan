use crate::FileIdentity;
use crate::runtime::ParallelRuntime;
use crate::walker::{ErrorPolicy, WalkEntry, WalkError, WalkOperation, WalkOptions};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, mpsc};

mod collect;
pub(crate) mod dynamic;
mod ordered_pull;
mod pull;
mod visit;
mod visit_worker;

use collect::{DirectoryTask, collect_lane, collect_serial, collect_shallow, expand_frontier};
pub use pull::ParallelWalkIter;
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
/// every worker. Link-following carries an immutable ancestry set with each
/// directory task so aliases remain parallel without losing cycle detection.
pub struct ParallelWalker {
    pub(super) root: PathBuf,
    pub(super) options: WalkOptions,
    pub(super) parallelism: usize,
    pub(super) skip_stdout: Option<FileIdentity>,
    pub(super) runtime: ParallelRuntime,
}

impl ParallelWalker {
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            options: WalkOptions::default(),
            parallelism: 0,
            skip_stdout: None,
            runtime: ParallelRuntime::global(),
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

    /// Selects the global, dedicated, or application-owned executor.
    #[must_use]
    pub fn runtime(mut self, runtime: ParallelRuntime) -> Self {
        self.runtime = runtime;
        self
    }

    /// Skips a regular file that refers to redirected standard output.
    ///
    /// This applies consistently to collected, visitor and pull APIs and
    /// prevents feedback loops when command output is written inside a walked
    /// tree.
    #[must_use]
    pub fn skip_stdout(mut self, enabled: bool) -> Self {
        self.skip_stdout = enabled.then(crate::stdout::identity).flatten();
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
        let skip_stdout = self.skip_stdout;
        if self.runtime.is_worker_thread() {
            return collect_serial(&self.root, self.options)
                .map(|report| without_stdout(report, skip_stdout));
        }
        if self.options.follow_links {
            return self
                .walk_dynamic()
                .map(|report| without_stdout(report, skip_stdout));
        }
        let mut shallow = collect_shallow(&self.root, self.options)?;
        if self.options.error_policy == ErrorPolicy::Abort && !shallow.errors.is_empty() {
            return Err(shallow.errors.into_iter().next().expect("error exists"));
        }
        if !self.options.same_file_system && shallow.tasks.len() < 2 {
            let target = requested_workers(&self.runtime, self.parallelism, self.options.max_open)
                .min(FRONTIER_TARGET_TASKS);
            shallow = expand_frontier(shallow, self.options, target);
        }
        if shallow.tasks.is_empty() {
            return Ok(without_stdout(
                ParallelWalkReport {
                    entries: shallow.entries,
                    errors: shallow.errors,
                },
                skip_stdout,
            ));
        }
        self.walk_lanes(shallow)
            .map(|report| without_stdout(report, skip_stdout))
    }

    fn walk_dynamic(self) -> Result<ParallelWalkReport, WalkError> {
        let report = Arc::new(Mutex::new(ParallelWalkReport {
            entries: Vec::new(),
            errors: Vec::new(),
        }));
        let visitor_report = Arc::clone(&report);
        dynamic::stream_batched(
            &self.root,
            self.options,
            self.parallelism,
            &self.runtime,
            &crate::CancellationToken::new(),
            move |entries, errors| {
                let mut report = visitor_report
                    .lock()
                    .expect("parallel walk report is not poisoned");
                report.entries.extend(entries);
                report.errors.extend(errors.iter().map(copy_walk_error));
                true
            },
        )?;
        let mut report = Arc::try_unwrap(report)
            .expect("parallel walk visitor released report")
            .into_inner()
            .expect("parallel walk report is not poisoned");
        report
            .entries
            .sort_unstable_by(|left, right| left.path().cmp(right.path()));
        report.errors.sort_unstable_by(|left, right| {
            left.path()
                .cmp(right.path())
                .then_with(|| left.depth().cmp(&right.depth()))
        });
        Ok(report)
    }

    fn walk_lanes(self, shallow: collect::ShallowWalk) -> Result<ParallelWalkReport, WalkError> {
        let mut entries = shallow.entries;
        let mut errors = shallow.errors;
        let task_count = shallow.tasks.len();
        let worker_count = parallel_worker_count(
            &self.runtime,
            self.parallelism,
            self.options.max_open,
            task_count,
        );
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
            self.runtime
                .try_execute(move || {
                    let report = collect_lane(lane, options, &root);
                    let _ = sender.send((index, report));
                })
                .map_err(|source| schedule_error(&self.root, source))?;
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

fn parallel_worker_count(
    runtime: &ParallelRuntime,
    parallelism: usize,
    max_open: usize,
    tasks: usize,
) -> usize {
    requested_workers(runtime, parallelism, max_open)
        .min(tasks)
        .max(1)
}

fn requested_workers(runtime: &ParallelRuntime, parallelism: usize, max_open: usize) -> usize {
    let available = runtime.parallelism();
    if parallelism == 0 {
        available.min(default_traversal_workers())
    } else {
        parallelism.min(available)
    }
    .min(max_open.max(1))
    .max(1)
}

fn schedule_error(root: &std::path::Path, source: std::io::Error) -> WalkError {
    WalkError::new(root, 0, WalkOperation::ScheduleWorker, source)
}

const fn default_traversal_workers() -> usize {
    if cfg!(windows) { 16 } else { 8 }
}

const FRONTIER_TARGET_TASKS: usize = 4;

fn copy_walk_error(error: &WalkError) -> WalkError {
    WalkError::new(
        error.path().to_path_buf(),
        error.depth(),
        error.operation(),
        std::io::Error::new(error.io_error().kind(), error.io_error().to_string()),
    )
}

pub(super) fn matches_stdout(entry: &WalkEntry, identity: Option<FileIdentity>) -> bool {
    identity.is_some_and(|identity| {
        entry.is_file() && crate::stdout::path_matches(entry.path(), identity).unwrap_or(false)
    })
}

fn without_stdout(
    mut report: ParallelWalkReport,
    identity: Option<FileIdentity>,
) -> ParallelWalkReport {
    if identity.is_some() {
        report
            .entries
            .retain(|entry| !matches_stdout(entry, identity));
    }
    report
}
