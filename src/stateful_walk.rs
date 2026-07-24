use crate::ParallelRuntime;
use crate::walk_platform::{DirectoryIdentity, FileSystemId};
use crate::walker::{WalkEntry, WalkError, WalkOptions, Walker};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

mod parallel;

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

impl<R, E> StatefulWalker<R, E>
where
    R: Clone + Send + 'static,
    E: Default + Send + 'static,
{
    fn new(builder: StatefulWalkBuilder<R, E>) -> Result<Self, WalkError> {
        let options = builder.options.normalized();
        let mut root_options = options;
        root_options.min_depth = 0;
        let mut walker = Walker::with_options(&builder.root, root_options)?;
        let root_file_system = walker.root_file_system;
        let root = Arc::clone(&walker.root);
        let root_entry = walker.next().expect("a validated root yields one entry")?;
        let identity = root_entry.directory_identity();
        let mut ancestors = HashSet::new();
        if let Some(identity) = identity {
            ancestors.insert(identity);
        }
        let can_descend = root_entry.is_dir() && root_entry.skip_reason().is_none();
        let root_entry = (root_entry.depth() >= options.min_depth).then(|| StatefulWalkEntry {
            read_children: can_descend,
            entry: root_entry,
            state: E::default(),
        });
        let pending = can_descend.then(|| DirectoryTask {
            path: root.as_ref().clone(),
            depth: 0,
            identity,
            ancestors,
            read_state: builder.root_read_dir_state,
        });
        Ok(Self {
            root,
            root_file_system,
            options,
            processor: builder.processor,
            root_entry,
            pending,
            frames: Vec::new(),
        })
    }

    fn read_directory(&self, mut task: DirectoryTask<R>) -> DirectoryFrame<R, E> {
        let mut worker_options = self.options;
        worker_options.error_policy = crate::ErrorPolicy::Continue;
        worker_options.min_depth = 0;
        worker_options.max_open = 1;
        worker_options.max_depth = Some(
            self.options
                .max_depth
                .unwrap_or(task.depth.saturating_add(1))
                .min(task.depth.saturating_add(1)),
        );
        let mut walker = Walker::from_known_directory_with_ancestry(
            &self.root,
            task.path.clone(),
            task.depth,
            worker_options,
            self.root_file_system,
            task.identity,
            task.ancestors.clone(),
        );
        let mut entries = Vec::new();
        while let Some(item) = walker.next() {
            match item {
                Ok(mut entry) => {
                    if entry.is_dir()
                        && entry.skip_reason() == Some(crate::WalkSkipReason::MaxDepth)
                        && self
                            .options
                            .max_depth
                            .is_none_or(|maximum| entry.depth() < maximum)
                    {
                        entry.clear_depth_skip();
                    }
                    if entry.is_dir() {
                        walker.skip_current_dir();
                    }
                    entries.push(Ok(StatefulWalkEntry {
                        read_children: entry.is_dir() && entry.skip_reason().is_none(),
                        entry,
                        state: E::default(),
                    }));
                }
                Err(error) => entries.push(Err(error)),
            }
        }
        if let Some(processor) = self.processor.as_ref() {
            processor(task.depth, &task.path, &mut task.read_state, &mut entries);
        }
        DirectoryFrame {
            entries: entries.into_iter(),
            child_state: task.read_state,
            ancestors: task.ancestors,
        }
    }
}

impl<R, E> Iterator for StatefulWalker<R, E>
where
    R: Clone + Send + 'static,
    E: Default + Send + 'static,
{
    type Item = Result<StatefulWalkEntry<E>, WalkError>;

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(root_entry) = self.root_entry.take() {
            return Some(Ok(root_entry));
        }
        loop {
            if let Some(task) = self.pending.take() {
                let frame = self.read_directory(task);
                self.frames.push(frame);
            }
            let frame = self.frames.last_mut()?;
            let Some(item) = frame.entries.next() else {
                self.frames.pop();
                continue;
            };
            if let Ok(entry) = &item
                && entry.read_children
            {
                let identity = entry.entry.directory_identity();
                let mut ancestors = frame.ancestors.clone();
                if let Some(identity) = identity {
                    ancestors.insert(identity);
                }
                self.pending = Some(DirectoryTask {
                    path: entry.path().to_path_buf(),
                    depth: entry.depth(),
                    identity,
                    ancestors,
                    read_state: frame.child_state.clone(),
                });
            }
            let visible = item
                .as_ref()
                .map_or(true, |entry| entry.depth() >= self.options.min_depth);
            if visible {
                return Some(item);
            }
        }
    }
}
