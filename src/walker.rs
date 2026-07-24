use crate::walk_builder::{EntryFilter, EntrySorter};
use crate::walk_platform::{
    DirectoryIdentity, FileSystemId, PlatformDirectoryInfo, directory_info,
};
pub use crate::walk_types::{
    ErrorPolicy, RootSymlinkPolicy, WalkEntry, WalkError, WalkOperation, WalkOptions,
    WalkSkipReason,
};
use std::collections::{HashSet, VecDeque};
use std::fs::{self, FileType};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Debug)]
pub(crate) struct PendingDirectory {
    pub(crate) path: PathBuf,
    pub(crate) depth: usize,
    pub(crate) identity: Option<DirectoryIdentity>,
    pub(crate) post_entry: Option<WalkEntry>,
}

// Keeping ReadDir inline avoids one heap allocation per directory on the hot
// traversal path; the stack is depth-bounded and buffered at max_open.
#[allow(clippy::large_enum_variant)]
pub(crate) enum DirectoryEntries {
    Open(fs::ReadDir),
    Buffered(VecDeque<io::Result<fs::DirEntry>>),
}

impl DirectoryEntries {
    pub(crate) fn next(&mut self) -> Option<io::Result<fs::DirEntry>> {
        match self {
            Self::Open(entries) => entries.next(),
            Self::Buffered(entries) => entries.pop_front(),
        }
    }

    pub(crate) const fn is_open(&self) -> bool {
        matches!(self, Self::Open(_))
    }
}

pub(crate) struct DirectoryFrame {
    pub(crate) path: PathBuf,
    pub(crate) depth: usize,
    pub(crate) entries: DirectoryEntries,
    pub(crate) identity: Option<DirectoryIdentity>,
    pub(crate) post_entry: Option<WalkEntry>,
}

/// Iterative depth-first filesystem walker.
///
/// Paths remain native `PathBuf` values; no lossy UTF-8 conversion occurs.
/// Open directory handles are bounded by `WalkOptions::max_open`; when a deep
/// tree reaches the limit, the oldest remaining directory entries are buffered
/// and its handle is closed.
#[allow(clippy::struct_excessive_bools)]
pub struct Walker {
    pub(crate) root: Arc<PathBuf>,
    pub(crate) root_components: usize,
    pub(crate) root_file_type: Option<FileType>,
    pub(crate) root_file_system: Option<FileSystemId>,
    pub(crate) root_directory_info: Option<PlatformDirectoryInfo>,
    pub(crate) options: WalkOptions,
    pub(crate) frames: Vec<DirectoryFrame>,
    pub(crate) open_handles: usize,
    pub(crate) yield_root: bool,
    pub(crate) pending_directory: Option<PendingDirectory>,
    pub(crate) skip_pending_directory: bool,
    pub(crate) active_directories: HashSet<DirectoryIdentity>,
    pub(crate) finished: bool,
    pub(crate) sorter: Option<EntrySorter>,
    pub(crate) filter: Option<EntryFilter>,
    pub(crate) contents_first: bool,
    pub(crate) deferred_entry: Option<WalkEntry>,
}

impl Walker {
    /// Creates a walker with the default traversal policy.
    ///
    /// # Errors
    ///
    /// Returns an error when the root cannot be resolved or inspected.
    pub fn new(root: impl AsRef<Path>) -> Result<Self, WalkError> {
        Self::with_options(root, WalkOptions::default())
    }

    /// Creates a walker with an explicit traversal policy.
    ///
    /// # Errors
    ///
    /// Returns an error when the root cannot be resolved or inspected.
    pub fn with_options(root: impl AsRef<Path>, options: WalkOptions) -> Result<Self, WalkError> {
        Self::with_behavior(root, options, None, None, false)
    }

    pub(crate) fn with_behavior(
        root: impl AsRef<Path>,
        options: WalkOptions,
        sorter: Option<EntrySorter>,
        filter: Option<EntryFilter>,
        contents_first: bool,
    ) -> Result<Self, WalkError> {
        let requested = root.as_ref();
        let options = options.normalized();
        if options.root_symlink_policy == RootSymlinkPolicy::Reject {
            let metadata = fs::symlink_metadata(requested).map_err(|source| {
                WalkError::new(requested, 0, WalkOperation::ReadMetadata, source)
            })?;
            if metadata.file_type().is_symlink() {
                return Err(WalkError::new(
                    requested,
                    0,
                    WalkOperation::ReadMetadata,
                    io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "root symlink rejected by policy",
                    ),
                ));
            }
        }
        let canonical = if options.follow_links || options.same_file_system {
            requested.canonicalize().map_err(|source| {
                WalkError::new(requested, 0, WalkOperation::Canonicalize, source)
            })?
        } else if requested.is_absolute() {
            requested.to_path_buf()
        } else {
            std::env::current_dir()
                .map_err(|source| {
                    WalkError::new(requested, 0, WalkOperation::Canonicalize, source)
                })?
                .join(requested)
        };
        let metadata = fs::metadata(&canonical)
            .map_err(|source| WalkError::new(&canonical, 0, WalkOperation::ReadMetadata, source))?;
        if !metadata.is_dir() {
            return Err(WalkError::new(
                &canonical,
                0,
                WalkOperation::ReadMetadata,
                io::Error::new(io::ErrorKind::InvalidInput, "root is not a directory"),
            ));
        }
        let root_directory_info = if options.follow_links || options.same_file_system {
            Some(directory_info(&canonical, &metadata).map_err(|source| {
                WalkError::new(&canonical, 0, WalkOperation::ReadMetadata, source)
            })?)
        } else {
            None
        };
        let root_file_system = if options.same_file_system {
            root_directory_info.map(|info| info.file_system)
        } else {
            None
        };
        let root = Arc::new(canonical.clone());
        Ok(Self {
            root: Arc::clone(&root),
            root_components: canonical.components().count(),
            root_file_type: Some(metadata.file_type()),
            root_file_system,
            root_directory_info,
            options,
            frames: Vec::new(),
            open_handles: 0,
            yield_root: true,
            pending_directory: None,
            skip_pending_directory: false,
            active_directories: HashSet::new(),
            finished: false,
            sorter,
            filter,
            contents_first,
            deferred_entry: None,
        })
    }

    pub(crate) fn from_known_directory(
        root: &Arc<PathBuf>,
        directory: PathBuf,
        depth: usize,
        options: WalkOptions,
        root_file_system: Option<FileSystemId>,
    ) -> Self {
        let options = options.normalized();
        let root_components = root.components().count();
        Self {
            root: Arc::clone(root),
            root_components,
            root_file_type: None,
            root_file_system,
            root_directory_info: None,
            options,
            frames: Vec::new(),
            open_handles: 0,
            yield_root: false,
            pending_directory: Some(PendingDirectory {
                path: directory,
                depth,
                identity: None,
                post_entry: None,
            }),
            skip_pending_directory: false,
            active_directories: HashSet::new(),
            finished: false,
            sorter: None,
            filter: None,
            contents_first: false,
            deferred_entry: None,
        }
    }

    #[must_use]
    pub fn root(&self) -> &Path {
        self.root.as_path()
    }

    #[must_use]
    pub const fn options(&self) -> &WalkOptions {
        &self.options
    }

    /// Prevents descent into the directory returned by the previous `next`.
    pub fn skip_current_dir(&mut self) {
        if self.pending_directory.is_some() {
            self.skip_pending_directory = true;
        }
    }

    pub(crate) fn schedule_pending_directory(&mut self) -> Option<WalkError> {
        let pending = self.pending_directory.take()?;
        if self.skip_pending_directory {
            self.skip_pending_directory = false;
            return None;
        }
        if self.open_handles >= self.options.max_open {
            self.buffer_oldest_open_directory();
        }
        match fs::read_dir(&pending.path) {
            Ok(entries) => {
                if let Some(identity) = pending.identity {
                    self.active_directories.insert(identity);
                }
                let (entries, opened) = match self.sorter.as_ref() {
                    None => (DirectoryEntries::Open(entries), true),
                    Some(sorter) => {
                        let mut entries = entries.collect::<Vec<_>>();
                        entries.sort_by(|left, right| match (left, right) {
                            (Ok(left), Ok(right)) => sorter(&left.file_name(), &right.file_name()),
                            (Err(_), Ok(_)) => std::cmp::Ordering::Less,
                            (Ok(_), Err(_)) => std::cmp::Ordering::Greater,
                            (Err(_), Err(_)) => std::cmp::Ordering::Equal,
                        });
                        (DirectoryEntries::Buffered(entries.into()), false)
                    }
                };
                self.frames.push(DirectoryFrame {
                    path: pending.path,
                    depth: pending.depth,
                    entries,
                    identity: pending.identity,
                    post_entry: pending.post_entry,
                });
                self.open_handles += usize::from(opened);
                None
            }
            Err(source) => {
                self.deferred_entry = pending.post_entry;
                Some(WalkError::new(
                    pending.path,
                    pending.depth,
                    WalkOperation::ReadDirectory,
                    source,
                ))
            }
        }
    }

    fn buffer_oldest_open_directory(&mut self) {
        let Some(index) = self.frames.iter().position(|frame| frame.entries.is_open()) else {
            return;
        };
        let placeholder = DirectoryEntries::Buffered(VecDeque::new());
        let entries = std::mem::replace(&mut self.frames[index].entries, placeholder);
        if let DirectoryEntries::Open(entries) = entries {
            self.frames[index].entries = DirectoryEntries::Buffered(entries.collect());
            self.open_handles -= 1;
        }
    }
}
