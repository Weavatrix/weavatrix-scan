use super::{ParallelMultiWalker, root_worker_count};
use crate::{
    CancellationToken, ParallelVisitReport, ParallelWalker, WalkControl, WalkError, WalkEvent,
};
use std::path::Path;
use std::sync::Arc;

/// A streaming event tagged with its root's insertion index and path.
#[derive(Debug)]
pub struct ParallelMultiWalkEvent<'a> {
    pub root_index: usize,
    pub root: &'a Path,
    pub event: WalkEvent<'a>,
}

/// Per-root streaming reports in root insertion order.
#[derive(Debug)]
pub struct ParallelMultiVisitReport {
    pub reports: Vec<ParallelVisitReport>,
}

impl ParallelMultiVisitReport {
    #[must_use]
    pub const fn len(&self) -> usize {
        self.reports.len()
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.reports.is_empty()
    }

    #[must_use]
    pub fn visited(&self) -> u64 {
        self.reports
            .iter()
            .fold(0_u64, |total, report| total.saturating_add(report.visited))
    }

    #[must_use]
    pub fn quit(&self) -> bool {
        self.reports.iter().any(|report| report.quit)
    }

    #[must_use]
    pub fn cancelled(&self) -> bool {
        self.reports.iter().any(|report| report.cancelled)
    }
}

impl ParallelMultiWalker {
    /// Streams every configured root through one concurrent callback.
    ///
    /// Callback order is intentionally unspecified. Each event carries its
    /// root insertion index, while the returned per-root reports preserve
    /// insertion order. `WalkControl::Quit` cancels every active root.
    ///
    /// # Errors
    ///
    /// Returns the first root error in insertion order after all started root
    /// workers have joined.
    ///
    /// # Panics
    ///
    /// Panics if the callback or an internal root worker panics.
    pub fn visit<F>(self, visitor: F) -> Result<ParallelMultiVisitReport, WalkError>
    where
        F: for<'entry> Fn(ParallelMultiWalkEvent<'entry>) -> WalkControl + Send + Sync + 'static,
    {
        self.visit_with_cancellation(&CancellationToken::new(), visitor)
    }

    /// Multi-root streaming traversal with shared cooperative cancellation.
    ///
    /// Returning `WalkControl::Quit` cancels the supplied token so every
    /// active and not-yet-started root observes the same stop request.
    ///
    /// # Errors
    ///
    /// Returns the first root error in insertion order after all started root
    /// workers have joined.
    ///
    /// # Panics
    ///
    /// Panics if the callback or an internal root worker panics.
    pub fn visit_with_cancellation<F>(
        self,
        cancellation: &CancellationToken,
        visitor: F,
    ) -> Result<ParallelMultiVisitReport, WalkError>
    where
        F: for<'entry> Fn(ParallelMultiWalkEvent<'entry>) -> WalkControl + Send + Sync + 'static,
    {
        let worker_count = if self.runtime.is_worker_thread() {
            1
        } else {
            root_worker_count(self.root_parallelism, self.roots.len())
        };
        let visitor = Arc::new(visitor);
        if worker_count <= 1 {
            let reports = self
                .roots
                .into_iter()
                .enumerate()
                .map(|(root_index, root)| {
                    visit_root(
                        root_index,
                        root,
                        self.options,
                        self.traversal_parallelism,
                        self.skip_stdout,
                        self.runtime.clone(),
                        cancellation,
                        Arc::clone(&visitor),
                    )
                })
                .collect::<Result<Vec<_>, _>>()?;
            return Ok(ParallelMultiVisitReport { reports });
        }

        let chunk_size = self.roots.len().div_ceil(worker_count);
        let indexed = self.roots.into_iter().enumerate().collect::<Vec<_>>();
        let mut visited = std::thread::scope(|scope| {
            indexed
                .chunks(chunk_size)
                .map(|chunk| {
                    let visitor = Arc::clone(&visitor);
                    let runtime = self.runtime.clone();
                    let cancellation = cancellation.clone();
                    scope.spawn(move || {
                        chunk
                            .iter()
                            .map(|(root_index, root)| {
                                (
                                    *root_index,
                                    visit_root(
                                        *root_index,
                                        root.clone(),
                                        self.options,
                                        self.traversal_parallelism,
                                        self.skip_stdout,
                                        runtime.clone(),
                                        &cancellation,
                                        Arc::clone(&visitor),
                                    ),
                                )
                            })
                            .collect::<Vec<_>>()
                    })
                })
                .collect::<Vec<_>>()
                .into_iter()
                .flat_map(|handle| handle.join().expect("multi-root streaming worker panicked"))
                .collect::<Vec<_>>()
        });
        visited.sort_unstable_by_key(|(index, _)| *index);
        let reports = visited
            .into_iter()
            .map(|(_, report)| report)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(ParallelMultiVisitReport { reports })
    }
}

#[allow(clippy::too_many_arguments)]
fn visit_root<F>(
    root_index: usize,
    root: std::path::PathBuf,
    options: crate::WalkOptions,
    traversal_parallelism: usize,
    skip_stdout: bool,
    runtime: crate::ParallelRuntime,
    cancellation: &CancellationToken,
    visitor: Arc<F>,
) -> Result<ParallelVisitReport, WalkError>
where
    F: for<'entry> Fn(ParallelMultiWalkEvent<'entry>) -> WalkControl + Send + Sync + 'static,
{
    let event_root = root.clone();
    let quit_cancellation = cancellation.clone();
    let result = ParallelWalker::new(root)
        .options(options)
        .with_parallelism(traversal_parallelism)
        .runtime(runtime)
        .skip_stdout(skip_stdout)
        .visit_with_cancellation(cancellation, move |event| {
            let control = visitor(ParallelMultiWalkEvent {
                root_index,
                root: &event_root,
                event,
            });
            if control == WalkControl::Quit {
                quit_cancellation.cancel();
            }
            control
        });
    if result.is_err() {
        cancellation.cancel();
    }
    result
}
