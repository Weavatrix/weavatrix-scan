use crate::config::ScanOptions;
use crate::error::Result;
use crate::report::ScanReport;
use crate::runtime::ParallelRuntime;
use crate::scanner::scan_repository_with_runtime;
use std::path::PathBuf;

/// Deterministic collection of independently rooted scan reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MultiScanReport {
    /// Reports remain in the same order as roots were added.
    pub reports: Vec<ScanReport>,
}

impl MultiScanReport {
    #[must_use]
    pub const fn len(&self) -> usize {
        self.reports.len()
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.reports.is_empty()
    }
}

/// Scans independent roots concurrently while preserving report order.
pub struct MultiScanner {
    roots: Vec<PathBuf>,
    options: ScanOptions,
    root_parallelism: usize,
    runtime: ParallelRuntime,
}

impl MultiScanner {
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            roots: vec![root.into()],
            options: ScanOptions::default(),
            root_parallelism: 0,
            runtime: ParallelRuntime::global(),
        }
    }

    #[must_use]
    pub fn add_root(mut self, root: impl Into<PathBuf>) -> Self {
        self.roots.push(root.into());
        self
    }

    #[must_use]
    pub fn options(mut self, options: ScanOptions) -> Self {
        self.options = options;
        self
    }

    /// Sets concurrent root scans. Zero selects bounded available parallelism.
    #[must_use]
    pub const fn with_root_parallelism(mut self, parallelism: usize) -> Self {
        self.root_parallelism = parallelism;
        self
    }

    /// Selects the executor shared by parallel discovery in every root.
    #[must_use]
    pub fn runtime(mut self, runtime: ParallelRuntime) -> Self {
        self.runtime = runtime;
        self
    }

    /// Scans all roots concurrently and returns them in insertion order.
    ///
    /// # Errors
    ///
    /// Returns the first error in insertion order after all already-started
    /// root scans have joined.
    ///
    /// # Panics
    ///
    /// Panics if an internal root worker panics.
    pub fn scan(self) -> Result<MultiScanReport> {
        let worker_count = if self.runtime.is_worker_thread() {
            1
        } else {
            root_worker_count(self.root_parallelism, self.roots.len())
        };
        if worker_count <= 1 {
            let reports = self
                .roots
                .iter()
                .map(|root| scan_repository_with_runtime(root, &self.options, None, &self.runtime))
                .collect::<Result<Vec<_>>>()?;
            return Ok(MultiScanReport { reports });
        }

        let chunk_size = self.roots.len().div_ceil(worker_count);
        let indexed = self.roots.into_iter().enumerate().collect::<Vec<_>>();
        let mut scanned = std::thread::scope(|scope| {
            indexed
                .chunks(chunk_size)
                .map(|chunk| {
                    let options = &self.options;
                    let runtime = self.runtime.clone();
                    scope.spawn(move || {
                        chunk
                            .iter()
                            .map(|(index, root)| {
                                (
                                    *index,
                                    scan_repository_with_runtime(root, options, None, &runtime),
                                )
                            })
                            .collect::<Vec<_>>()
                    })
                })
                .collect::<Vec<_>>()
                .into_iter()
                .flat_map(|handle| handle.join().expect("multi-root scanner worker panicked"))
                .collect::<Vec<_>>()
        });
        scanned.sort_unstable_by_key(|(index, _)| *index);
        let reports = scanned
            .into_iter()
            .map(|(_, report)| report)
            .collect::<Result<Vec<_>>>()?;
        Ok(MultiScanReport { reports })
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
