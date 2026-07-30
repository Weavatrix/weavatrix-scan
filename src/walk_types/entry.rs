use super::{DirectoryIdentity, FileVersion, OsStr, Path, PathBuf, WalkEntry, WalkSkipReason};

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
