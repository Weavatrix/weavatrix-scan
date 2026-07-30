use crate::runtime::ParallelRuntime;
use crate::walk_types::{DirectoryIdentity, FileSystemId, WalkEntry, WalkError, WalkOptions};
use crate::walker::Walker;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

mod parallel;
mod serial;

pub use parallel::ParallelStatefulWalker;

type DirectoryProcessor<R, E> = Arc<
    dyn Fn(usize, &Path, &mut R, &mut Vec<Result<StatefulWalkEntry<E>, WalkError>>)
        + Send
        + Sync
        + 'static,
>;

/// A walk entry carrying caller-owned state assigned by `process_read_dir`.
#[derive(Debug)]
pub struct StatefulWalkEntry<E> {
    entry: WalkEntry,
    /// State retained with this entry after the directory callback returns.
    pub state: E,
    read_children: bool,
}

impl<E> StatefulWalkEntry<E> {
    #[must_use]
    pub const fn entry(&self) -> &WalkEntry {
        &self.entry
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        self.entry.path()
    }

    #[must_use]
    pub const fn depth(&self) -> usize {
        self.entry.depth()
    }

    #[must_use]
    pub const fn is_file(&self) -> bool {
        self.entry.is_file()
    }

    #[must_use]
    pub const fn is_dir(&self) -> bool {
        self.entry.is_dir()
    }

    #[must_use]
    pub const fn read_children(&self) -> bool {
        self.read_children
    }

    /// Controls whether traversal descends into this directory.
    pub const fn set_read_children(&mut self, enabled: bool) {
        self.read_children = enabled && self.entry.is_dir() && self.entry.skip_reason().is_none();
    }

    #[must_use]
    pub fn into_parts(self) -> (WalkEntry, E) {
        (self.entry, self.state)
    }
}

/// Builder for an iterative DFS walk with per-directory and per-entry state.
///
/// `R` is cloned from a processed directory into each accepted child
/// directory. `E` is attached independently to every yielded entry.
pub struct StatefulWalkBuilder<R, E> {
    root: PathBuf,
    options: WalkOptions,
    root_read_dir_state: R,
    processor: Option<DirectoryProcessor<R, E>>,
    parallelism: usize,
    runtime: ParallelRuntime,
}

impl<R, E> StatefulWalkBuilder<R, E>
where
    R: Clone + Send + 'static,
    E: Default + Send + 'static,
{
    #[must_use]
    pub fn new(root: impl Into<PathBuf>, root_read_dir_state: R) -> Self {
        Self {
            root: root.into(),
            options: WalkOptions::default(),
            root_read_dir_state,
            processor: None,
            parallelism: 0,
            runtime: ParallelRuntime::global(),
        }
    }

    #[must_use]
    pub const fn options(mut self, options: WalkOptions) -> Self {
        self.options = options;
        self
    }

    /// Sets parallel directory workers. Zero selects runtime parallelism.
    #[must_use]
    pub const fn with_parallelism(mut self, parallelism: usize) -> Self {
        self.parallelism = parallelism;
        self
    }

    /// Selects the executor used by parallel stateful traversal.
    #[must_use]
    pub fn runtime(mut self, runtime: ParallelRuntime) -> Self {
        self.runtime = runtime;
        self
    }

    /// Processes the complete immediate child batch before entries are yielded.
    ///
    /// The callback may sort or retain the vector, remove local errors, mutate
    /// the inherited read-directory state, attach state to entries, and call
    /// [`StatefulWalkEntry::set_read_children`] for per-entry pruning.
    #[must_use]
    pub fn process_read_dir<F>(mut self, processor: F) -> Self
    where
        F: Fn(usize, &Path, &mut R, &mut Vec<Result<StatefulWalkEntry<E>, WalkError>>)
            + Send
            + Sync
            + 'static,
    {
        self.processor = Some(Arc::new(processor));
        self
    }

    /// Builds the stateful iterator after validating the root.
    ///
    /// # Errors
    ///
    /// Returns an error when the root cannot be resolved or inspected.
    pub fn build(self) -> Result<StatefulWalker<R, E>, WalkError> {
        StatefulWalker::new(self)
    }

    /// Builds a bounded parallel iterator with strict depth-first output.
    ///
    /// Directory callbacks run on the selected executor over complete child
    /// batches. Their mutated directory state is cloned into accepted child
    /// tasks, while yielded entries retain callback-assigned entry state.
    ///
    /// # Errors
    ///
    /// Returns a root-validation, coordinator-startup, or worker-submission
    /// error.
    pub fn build_parallel_ordered(
        self,
        capacity: usize,
    ) -> Result<ParallelStatefulWalker<E>, WalkError> {
        ParallelStatefulWalker::start(self, capacity)
    }
}

struct DirectoryTask<R> {
    path: PathBuf,
    depth: usize,
    identity: Option<DirectoryIdentity>,
    ancestors: HashSet<DirectoryIdentity>,
    read_state: R,
}

struct DirectoryFrame<R, E> {
    entries: std::vec::IntoIter<Result<StatefulWalkEntry<E>, WalkError>>,
    child_state: R,
    ancestors: HashSet<DirectoryIdentity>,
}

/// Strict depth-first iterator produced by [`StatefulWalkBuilder`].
pub struct StatefulWalker<R, E> {
    root: Arc<PathBuf>,
    root_file_system: Option<FileSystemId>,
    options: WalkOptions,
    processor: Option<DirectoryProcessor<R, E>>,
    root_entry: Option<StatefulWalkEntry<E>>,
    pending: Option<DirectoryTask<R>>,
    frames: Vec<DirectoryFrame<R, E>>,
}
