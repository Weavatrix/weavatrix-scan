use crate::{ParallelWalkReport, ParallelWalker, WalkError, WalkOptions};
use std::path::PathBuf;

/// Collected raw walk reports for independent roots in insertion order.
#[derive(Debug)]
pub struct ParallelMultiWalkReport {
    pub reports: Vec<ParallelWalkReport>,
}

impl ParallelMultiWalkReport {
    #[must_use]
    pub const fn len(&self) -> usize {
        self.reports.len()
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.reports.is_empty()
    }
}

/// Walks independent raw roots concurrently while preserving root order.
pub struct ParallelMultiWalker {
    roots: Vec<PathBuf>,
    options: WalkOptions,
    root_parallelism: usize,
    traversal_parallelism: usize,
}

impl ParallelMultiWalker {
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            roots: vec![root.into()],
            options: WalkOptions::default(),
            root_parallelism: 0,
            traversal_parallelism: 0,
        }
    }

    #[must_use]
    pub fn add_root(mut self, root: impl Into<PathBuf>) -> Self {
        self.roots.push(root.into());
        self
    }

    #[must_use]
    pub const fn options(mut self, options: WalkOptions) -> Self {
        self.options = options;
        self
    }

    /// Sets concurrently active roots. Zero uses available parallelism.
    #[must_use]
    pub const fn with_root_parallelism(mut self, parallelism: usize) -> Self {
        self.root_parallelism = parallelism;
        self
    }

    /// Sets directory workers requested by each active root.
    #[must_use]
    pub const fn with_traversal_parallelism(mut self, parallelism: usize) -> Self {
        self.traversal_parallelism = parallelism;
        self
    }

    /// Walks every root and returns reports in insertion order.
    ///
    /// # Errors
    ///
    /// Returns the first root error in insertion order after all started root
    /// workers have joined.
    ///
    /// # Panics
    ///
    /// Panics if an internal root worker panics.
    pub fn walk(self) -> Result<ParallelMultiWalkReport, WalkError> {
        let worker_count = if crate::pool::ThreadPool::is_worker_thread() {
            1
        } else {
            root_worker_count(self.root_parallelism, self.roots.len())
        };
        if worker_count <= 1 {
            let reports = self
                .roots
                .into_iter()
                .map(|root| {
                    ParallelWalker::new(root)
                        .options(self.options)
                        .with_parallelism(self.traversal_parallelism)
                        .walk()
                })
                .collect::<Result<Vec<_>, _>>()?;
            return Ok(ParallelMultiWalkReport { reports });
        }

        let chunk_size = self.roots.len().div_ceil(worker_count);
        let indexed = self.roots.into_iter().enumerate().collect::<Vec<_>>();
        let mut walked = std::thread::scope(|scope| {
            indexed
                .chunks(chunk_size)
                .map(|chunk| {
                    scope.spawn(|| {
                        chunk
                            .iter()
                            .map(|(index, root)| {
                                (
                                    *index,
                                    ParallelWalker::new(root)
                                        .options(self.options)
                                        .with_parallelism(self.traversal_parallelism)
                                        .walk(),
                                )
                            })
                            .collect::<Vec<_>>()
                    })
                })
                .collect::<Vec<_>>()
                .into_iter()
                .flat_map(|handle| handle.join().expect("multi-root walk worker panicked"))
                .collect::<Vec<_>>()
        });
        walked.sort_unstable_by_key(|(index, _)| *index);
        let reports = walked
            .into_iter()
            .map(|(_, report)| report)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(ParallelMultiWalkReport { reports })
    }
}

fn root_worker_count(requested: usize, roots: usize) -> usize {
    if roots == 0 {
        return 1;
    }
    let available = std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get);
    let requested = if requested == 0 {
        available.min(if cfg!(windows) { 8 } else { 16 })
    } else {
        requested.min(available)
    };
    requested.min(roots).max(1)
}
