use super::{
    Arc, DirectoryEntries, DirectoryFrame, DirectoryIdentity, EntryFilter, EntrySorter,
    FileSystemId, HashSet, Path, PathBuf, PendingDirectory, RootSymlinkPolicy, VecDeque, WalkError,
    WalkOperation, WalkOptions, Walker, directory_info, fs, io,
};

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
        Self::with_behavior(root, options, None, None, None, false)
    }

    pub(crate) fn with_behavior(
        root: impl AsRef<Path>,
        options: WalkOptions,
        sorter: Option<EntrySorter>,
        filter: Option<EntryFilter>,
        skip_stdout: Option<crate::report::FileIdentity>,
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
        let root_directory_info =
            if metadata.is_dir() && (options.follow_links || options.same_file_system) {
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
        let (root_bytes, root_version) = if options.collect_metadata && metadata.is_file() {
            (
                Some(metadata.len()),
                Some(crate::file_version::from_metadata(&metadata)),
            )
        } else {
            (None, None)
        };
        let plain_entries = !options.follow_links
            && !options.same_file_system
            && !options.collect_metadata
            && options.max_depth.is_none()
            && options.min_depth == 0
            && filter.is_none()
            && skip_stdout.is_none()
            && !contents_first;
        let root = Arc::new(canonical.clone());
        Ok(Self {
            root: Arc::clone(&root),
            root_components: canonical.components().count(),
            root_file_type: Some(metadata.file_type()),
            root_bytes,
            root_version,
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
            skip_stdout,
            contents_first,
            deferred_entry: None,
            plain_entries,
        })
    }

    pub(crate) fn from_known_directory(
        root: &Arc<PathBuf>,
        directory: PathBuf,
        depth: usize,
        options: WalkOptions,
        root_file_system: Option<FileSystemId>,
    ) -> Self {
        Self::from_known_directory_with_ancestry(
            root,
            directory,
            depth,
            options,
            root_file_system,
            None,
            HashSet::new(),
        )
    }

    pub(crate) fn from_known_directory_with_ancestry(
        root: &Arc<PathBuf>,
        directory: PathBuf,
        depth: usize,
        options: WalkOptions,
        root_file_system: Option<FileSystemId>,
        directory_identity: Option<DirectoryIdentity>,
        active_directories: HashSet<DirectoryIdentity>,
    ) -> Self {
        let options = options.normalized();
        let root_components = root.components().count();
        let plain_entries = !options.follow_links
            && !options.same_file_system
            && !options.collect_metadata
            && options.max_depth.is_none()
            && options.min_depth == 0;
        Self {
            root: Arc::clone(root),
            root_components,
            root_file_type: None,
            root_bytes: None,
            root_version: None,
            root_file_system,
            root_directory_info: None,
            options,
            frames: Vec::new(),
            open_handles: 0,
            yield_root: false,
            pending_directory: Some(PendingDirectory {
                path: directory,
                depth,
                identity: directory_identity,
                post_entry: None,
            }),
            skip_pending_directory: false,
            active_directories,
            finished: false,
            sorter: None,
            filter: None,
            skip_stdout: None,
            contents_first: false,
            deferred_entry: None,
            plain_entries,
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
                            (Ok(left), Ok(right)) => sorter(left, right),
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
