use crate::FileIdentity;
use crate::walker::{WalkEntry, WalkError, WalkOptions, Walker};
use std::cmp::Ordering;
use std::ffi::OsStr;
use std::fs::DirEntry;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};

pub(crate) type EntrySorter = Arc<dyn Fn(&DirEntry, &DirEntry) -> Ordering + Send + Sync + 'static>;
pub(crate) type EntryFilter = Arc<dyn Fn(&WalkEntry) -> bool + Send + Sync + 'static>;

/// Configures flexible single- or multi-root walking.
pub struct WalkBuilder {
    roots: Vec<PathBuf>,
    options: WalkOptions,
    sorter: Option<EntrySorter>,
    filter: Option<EntryFilter>,
    skip_stdout: Option<FileIdentity>,
    contents_first: bool,
}

impl WalkBuilder {
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            roots: vec![root.into()],
            options: WalkOptions::default(),
            sorter: None,
            filter: None,
            skip_stdout: None,
            contents_first: false,
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

    /// Sorts each directory by its native file name before descent.
    #[must_use]
    pub fn sort_by_file_name(mut self) -> Self {
        self.sorter = Some(Arc::new(|left, right| {
            left.file_name().cmp(&right.file_name())
        }));
        self
    }

    /// Sorts each directory with a comparator over complete directory entries.
    #[must_use]
    pub fn sort_by<F>(mut self, compare: F) -> Self
    where
        F: Fn(&DirEntry, &DirEntry) -> Ordering + Send + Sync + 'static,
    {
        self.sorter = Some(Arc::new(compare));
        self
    }

    /// Sorts with a native-name comparator without lossy UTF-8 conversion.
    #[must_use]
    pub fn sort_by_name<F>(mut self, compare: F) -> Self
    where
        F: Fn(&OsStr, &OsStr) -> Ordering + Send + Sync + 'static,
    {
        self.sorter = Some(Arc::new(move |left, right| {
            compare(&left.file_name(), &right.file_name())
        }));
        self
    }

    /// Skips a regular file that refers to redirected standard output.
    ///
    /// This prevents feedback loops such as scanning a tree while writing
    /// command output into a file inside that same tree.
    #[must_use]
    pub fn skip_stdout(mut self, enabled: bool) -> Self {
        self.skip_stdout = enabled.then(crate::stdout::identity).flatten();
        self
    }

    /// Emits a directory after all accepted descendants.
    #[must_use]
    pub const fn contents_first(mut self, enabled: bool) -> Self {
        self.contents_first = enabled;
        self
    }

    /// Filters entries before emission and prunes rejected directories.
    #[must_use]
    pub fn filter_entry<F>(mut self, filter: F) -> Self
    where
        F: Fn(&WalkEntry) -> bool + Send + Sync + 'static,
    {
        self.filter = Some(Arc::new(filter));
        self
    }

    /// Invokes a directory predicate before descent; files remain accepted.
    #[must_use]
    pub fn filter_directories<F>(mut self, filter: F) -> Self
    where
        F: Fn(&WalkEntry) -> bool + Send + Sync + 'static,
    {
        self.filter = Some(Arc::new(move |entry| !entry.is_dir() || filter(entry)));
        self
    }

    /// Invokes one mutable directory predicate across every configured root.
    ///
    /// The callback is serialized behind a mutex so captured state remains
    /// coherent if builder internals gain concurrent root traversal later.
    #[must_use]
    pub fn filter_directories_stateful<F>(mut self, filter: F) -> Self
    where
        F: FnMut(&WalkEntry) -> bool + Send + 'static,
    {
        let filter = Mutex::new(filter);
        self.filter = Some(Arc::new(move |entry| {
            if !entry.is_dir() {
                return true;
            }
            let mut filter = filter.lock().unwrap_or_else(PoisonError::into_inner);
            filter(entry)
        }));
        self
    }

    #[must_use]
    pub fn build(self) -> MultiWalker {
        MultiWalker {
            roots: self.roots.into_iter(),
            options: self.options,
            sorter: self.sorter,
            filter: self.filter,
            skip_stdout: self.skip_stdout,
            contents_first: self.contents_first,
            current: None,
        }
    }
}

/// Iterator returned by [`WalkBuilder`].
pub struct MultiWalker {
    roots: std::vec::IntoIter<PathBuf>,
    options: WalkOptions,
    sorter: Option<EntrySorter>,
    filter: Option<EntryFilter>,
    skip_stdout: Option<FileIdentity>,
    contents_first: bool,
    current: Option<Walker>,
}

impl Iterator for MultiWalker {
    type Item = Result<WalkEntry, WalkError>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some(walker) = self.current.as_mut()
                && let Some(item) = walker.next()
            {
                return Some(item);
            }
            self.current = None;
            let root = self.roots.next()?;
            match Walker::with_behavior(
                root,
                self.options,
                self.sorter.clone(),
                self.filter.clone(),
                self.skip_stdout,
                self.contents_first,
            ) {
                Ok(walker) => self.current = Some(walker),
                Err(error) => return Some(Err(error)),
            }
        }
    }
}
