use crate::report::FileVersion;
use crate::walk_platform::DirectoryIdentity;
use std::ffi::OsStr;
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorPolicy {
    Continue,
    Abort,
}

/// Controls whether the explicitly supplied root itself may be a symlink.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RootSymlinkPolicy {
    /// Resolve or traverse the requested root for backward compatibility.
    Follow,
    /// Reject a symlink at the final root path component.
    Reject,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WalkOptions {
    pub min_depth: usize,
    pub max_depth: Option<usize>,
    pub max_open: usize,
    pub same_file_system: bool,
    pub follow_links: bool,
    pub collect_metadata: bool,
    pub error_policy: ErrorPolicy,
    pub root_symlink_policy: RootSymlinkPolicy,
}

impl Default for WalkOptions {
    fn default() -> Self {
        Self {
            min_depth: 0,
            max_depth: None,
            max_open: Self::DEFAULT_MAX_OPEN,
            same_file_system: false,
            follow_links: false,
            collect_metadata: false,
            error_policy: ErrorPolicy::Continue,
            root_symlink_policy: RootSymlinkPolicy::Follow,
        }
    }
}

impl WalkOptions {
    /// Default hard upper bound for simultaneously open directory handles.
    pub const DEFAULT_MAX_OPEN: usize = 64;

    #[must_use]
    pub const fn with_min_depth(mut self, min_depth: usize) -> Self {
        self.min_depth = min_depth;
        self
    }

    #[must_use]
    pub const fn with_max_depth(mut self, max_depth: Option<usize>) -> Self {
        self.max_depth = max_depth;
        self
    }

    #[must_use]
    pub const fn with_max_open(mut self, max_open: usize) -> Self {
        self.max_open = if max_open == 0 { 1 } else { max_open };
        self
    }

    #[must_use]
    pub const fn with_same_file_system(mut self, enabled: bool) -> Self {
        self.same_file_system = enabled;
        self
    }

    #[must_use]
    pub const fn with_follow_links(mut self, enabled: bool) -> Self {
        self.follow_links = enabled;
        self
    }

    #[must_use]
    pub const fn with_metadata(mut self, enabled: bool) -> Self {
        self.collect_metadata = enabled;
        self
    }

    #[must_use]
    pub const fn with_error_policy(mut self, policy: ErrorPolicy) -> Self {
        self.error_policy = policy;
        self
    }

    #[must_use]
    pub const fn with_root_symlink_policy(mut self, policy: RootSymlinkPolicy) -> Self {
        self.root_symlink_policy = policy;
        self
    }

    pub(crate) fn normalized(mut self) -> Self {
        self.max_open = self.max_open.max(1);
        if let Some(max_depth) = self.max_depth
            && self.min_depth > max_depth
        {
            self.min_depth = max_depth;
        }
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WalkOperation {
    Canonicalize,
    ReadDirectory,
    ReadEntry,
    ReadMetadata,
    ScheduleWorker,
}

#[derive(Debug)]
pub struct WalkError {
    pub(crate) path: PathBuf,
    pub(crate) depth: usize,
    pub(crate) operation: WalkOperation,
    pub(crate) source: io::Error,
}

impl WalkError {
    pub(crate) fn new(
        path: impl Into<PathBuf>,
        depth: usize,
        operation: WalkOperation,
        source: io::Error,
    ) -> Self {
        Self {
            path: path.into(),
            depth,
            operation,
            source,
        }
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    #[must_use]
    pub const fn depth(&self) -> usize {
        self.depth
    }

    #[must_use]
    pub const fn operation(&self) -> WalkOperation {
        self.operation
    }

    #[must_use]
    pub const fn io_error(&self) -> &io::Error {
        &self.source
    }

    pub(crate) fn into_parts(self) -> (PathBuf, io::Error) {
        (self.path, self.source)
    }

    pub(crate) fn rebase_depth(&mut self, depth_offset: usize) {
        self.depth += depth_offset;
    }
}

impl fmt::Display for WalkError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{} at depth {} for {}: {}",
            operation_name(self.operation),
            self.depth,
            self.path.display(),
            self.source
        )
    }
}

impl std::error::Error for WalkError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.source)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WalkSkipReason {
    MaxDepth,
    FileSystemBoundary,
    PathEscape,
    SymlinkLoop,
}

#[derive(Debug, Clone)]
pub struct WalkEntry {
    pub(crate) root_components: usize,
    pub(crate) path: PathBuf,
    pub(crate) depth: usize,
    pub(crate) is_file: bool,
    pub(crate) is_directory: bool,
    pub(crate) is_symlink: bool,
    pub(crate) bytes: Option<u64>,
    pub(crate) version: Option<FileVersion>,
    pub(crate) hidden: Option<bool>,
    pub(crate) directory_identity: Option<DirectoryIdentity>,
    pub(crate) skip_reason: Option<WalkSkipReason>,
}

impl WalkEntry {
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Consumes the entry and returns its owned path without cloning.
    #[must_use]
    pub fn into_path(self) -> PathBuf {
        self.path
    }

    #[must_use]
    pub fn relative_path(&self) -> &Path {
        let mut components = self.path.components();
        for _ in 0..self.root_components {
            if components.next().is_none() {
                return self.path.as_path();
            }
        }
        components.as_path()
    }

    #[must_use]
    pub fn file_name(&self) -> &OsStr {
        self.path
            .file_name()
            .unwrap_or_else(|| self.path.as_os_str())
    }

    #[must_use]
    pub const fn depth(&self) -> usize {
        self.depth
    }

    #[must_use]
    pub const fn is_file(&self) -> bool {
        self.is_file
    }

    #[must_use]
    pub const fn is_dir(&self) -> bool {
        self.is_directory
    }

    #[must_use]
    pub const fn is_symlink(&self) -> bool {
        self.is_symlink
    }

    #[must_use]
    pub const fn bytes(&self) -> Option<u64> {
        self.bytes
    }

    /// Snapshot evidence captured with file metadata, when requested.
    #[must_use]
    pub const fn version(&self) -> Option<FileVersion> {
        self.version
    }

    pub(crate) const fn hidden(&self) -> Option<bool> {
        self.hidden
    }

    #[must_use]
    pub const fn skip_reason(&self) -> Option<WalkSkipReason> {
        self.skip_reason
    }

    pub(crate) fn rebase(&mut self, root: &Path, depth_offset: usize) {
        self.root_components = root.components().count();
        self.depth += depth_offset;
    }

    pub(crate) fn clear_depth_skip(&mut self) {
        if self.skip_reason == Some(WalkSkipReason::MaxDepth) {
            self.skip_reason = None;
        }
    }

    pub(crate) const fn directory_identity(&self) -> Option<DirectoryIdentity> {
        self.directory_identity
    }
}

const fn operation_name(operation: WalkOperation) -> &'static str {
    match operation {
        WalkOperation::Canonicalize => "canonicalize",
        WalkOperation::ReadDirectory => "read directory",
        WalkOperation::ReadEntry => "read entry",
        WalkOperation::ReadMetadata => "read metadata",
        WalkOperation::ScheduleWorker => "schedule worker",
    }
}
