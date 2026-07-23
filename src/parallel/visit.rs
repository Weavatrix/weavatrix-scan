use super::collect::{DirectoryTask, collect_shallow};
use super::visit_worker::{visit_lane, visit_serial};
use super::{ParallelWalker, parallel_worker_count};
use crate::control::CancellationToken;
use crate::pool::ThreadPool;
use crate::walker::{ErrorPolicy, WalkEntry, WalkError};
use std::collections::HashSet;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc,
};

/// A streaming event emitted from a parallel traversal worker.
#[derive(Debug)]
pub enum WalkEvent<'a> {
    Entry(&'a WalkEntry),
    Error(&'a WalkError),
}

/// Controls traversal after a streaming visitor handles an event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WalkControl {
    Continue,
    /// Prevents descent when the current event is a directory entry.
    Skip,
    /// Cooperatively stops every traversal worker.
    Quit,
}

/// Summary of a streaming parallel traversal.
#[derive(Debug)]
pub struct ParallelVisitReport {
    pub visited: u64,
    pub errors: Vec<WalkError>,
    pub quit: bool,
    pub cancelled: bool,
}

impl ParallelWalker {
    /// Visits entries directly on traversal workers without collecting paths.
    ///
    /// Visitor calls may run concurrently and their order is intentionally
    /// unspecified. Use [`Self::walk`] when deterministic collected order is
    /// required.
    ///
    /// # Errors
    ///
    /// Returns a root error or the first traversal error under
    /// `ErrorPolicy::Abort`.
    ///
    /// # Panics
    ///
    /// Panics if the visitor or an internal traversal worker panics.
    pub fn visit<F>(self, visitor: F) -> Result<ParallelVisitReport, WalkError>
    where
        F: for<'entry> Fn(WalkEvent<'entry>) -> WalkControl + Send + Sync + 'static,
    {
        self.visit_with_cancellation(&CancellationToken::new(), visitor)
    }

    /// Streaming parallel traversal with cooperative cancellation.
    ///
    /// # Errors
    ///
    /// Returns a root error or the first traversal error under
    /// `ErrorPolicy::Abort`.
    ///
    /// # Panics
    ///
    /// Panics if the visitor or an internal traversal worker panics.
    pub fn visit_with_cancellation<F>(
        self,
        cancellation: &CancellationToken,
        visitor: F,
    ) -> Result<ParallelVisitReport, WalkError>
    where
        F: for<'entry> Fn(WalkEvent<'entry>) -> WalkControl + Send + Sync + 'static,
    {
        if self.options.follow_links {
            return visit_serial(&self.root, self.options, cancellation, visitor);
        }
        let mut shallow = collect_shallow(&self.root, self.options)?;
        let stop = Arc::new(AtomicBool::new(false));
        let visitor = Arc::new(visitor);
        let mut visited = 0_u64;
        let mut errors = Vec::new();
        let mut skipped = HashSet::new();
        for entry in &shallow.entries {
            if cancellation.is_cancelled() || stop.load(Ordering::Acquire) {
                break;
            }
            visited = visited.saturating_add(1);
            match visitor(WalkEvent::Entry(entry)) {
                WalkControl::Skip if entry.depth() == 0 => {
                    stop.store(true, Ordering::Release);
                }
                WalkControl::Skip if entry.is_dir() => {
                    skipped.insert(entry.path().to_path_buf());
                }
                WalkControl::Continue | WalkControl::Skip => {}
                WalkControl::Quit => stop.store(true, Ordering::Release),
            }
        }
        shallow.tasks.retain(|task| !skipped.contains(&task.path));
        for error in shallow.errors {
            if cancellation.is_cancelled() || stop.load(Ordering::Acquire) {
                break;
            }
            let control = visitor(WalkEvent::Error(&error));
            let abort = self.options.error_policy == ErrorPolicy::Abort;
            errors.push(error);
            if abort || control == WalkControl::Quit {
                stop.store(true, Ordering::Release);
            }
        }
        if self.options.error_policy == ErrorPolicy::Abort && !errors.is_empty() {
            return Err(errors.remove(0));
        }
        if shallow.tasks.is_empty() || cancellation.is_cancelled() || stop.load(Ordering::Acquire) {
            return Ok(ParallelVisitReport {
                visited,
                errors,
                quit: stop.load(Ordering::Acquire),
                cancelled: cancellation.is_cancelled(),
            });
        }

        let worker_count =
            parallel_worker_count(self.parallelism, self.options.max_open, shallow.tasks.len());
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
            let visitor = Arc::clone(&visitor);
            let cancellation = cancellation.clone();
            let stop = Arc::clone(&stop);
            let options = self.options;
            ThreadPool::global().execute(move || {
                let report =
                    visit_lane(lane, options, &root, &cancellation, &stop, visitor.as_ref());
                let _ = sender.send((index, report));
            });
        }
        drop(sender);
        let mut completed = (0..worker_count).map(|_| None).collect::<Vec<_>>();
        for (index, report) in receiver {
            completed[index] = Some(report);
        }
        for report in completed {
            let report = report.expect("every parallel visitor lane reports completion");
            visited = visited.saturating_add(report.visited);
            errors.extend(report.errors);
        }
        if self.options.error_policy == ErrorPolicy::Abort && !errors.is_empty() {
            return Err(errors.remove(0));
        }
        Ok(ParallelVisitReport {
            visited,
            errors,
            quit: stop.load(Ordering::Acquire),
            cancelled: cancellation.is_cancelled(),
        })
    }
}
