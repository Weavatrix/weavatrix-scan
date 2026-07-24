use super::ParallelWalker;
use super::dynamic;
use super::visit_worker::visit_serial;
use crate::control::CancellationToken;
use crate::walker::{WalkEntry, WalkError};

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
        mut self,
        cancellation: &CancellationToken,
        visitor: F,
    ) -> Result<ParallelVisitReport, WalkError>
    where
        F: for<'entry> Fn(WalkEvent<'entry>) -> WalkControl + Send + Sync + 'static,
    {
        self.options = self.options.normalized();
        if crate::pool::ThreadPool::is_worker_thread() {
            return visit_serial(&self.root, self.options, cancellation, visitor);
        }
        dynamic::visit(
            &self.root,
            self.options,
            self.parallelism,
            cancellation,
            visitor,
        )
    }
}
