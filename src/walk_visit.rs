use crate::walk_platform::directory_info;
use crate::walk_types::{ErrorPolicy, WalkEntry, WalkError, WalkOperation, WalkSkipReason};
use crate::walker::{PendingDirectory, Walker};
use std::fs::{self, FileType};
use std::path::{Path, PathBuf};

impl Walker {
    pub(crate) fn visit(
        &mut self,
        path: PathBuf,
        depth: usize,
        file_type: Option<FileType>,
        mut bytes: Option<u64>,
    ) -> Result<WalkEntry, WalkError> {
        let file_type = entry_file_type(&path, depth, file_type)?;
        let is_symlink = file_type.is_symlink();
        let mut is_file = file_type.is_file();
        let mut is_directory = file_type.is_dir();
        let mut skip_reason = None;
        let mut directory_identity = None;

        let target_metadata = if is_symlink && self.options.follow_links {
            let metadata = fs::metadata(&path).map_err(|source| {
                WalkError::new(&path, depth, WalkOperation::ReadMetadata, source)
            })?;
            is_file = metadata.is_file();
            is_directory = metadata.is_dir();
            if self.options.collect_metadata && is_file {
                bytes = Some(metadata.len());
            }
            Some(metadata)
        } else {
            None
        };

        if is_symlink && !self.options.follow_links {
            is_file = false;
            is_directory = false;
        } else if is_symlink
            || (is_directory && (self.options.same_file_system || self.options.follow_links))
        {
            let canonical = if depth == 0 {
                self.root.as_ref().clone()
            } else {
                path.canonicalize().map_err(|source| {
                    WalkError::new(&path, depth, WalkOperation::Canonicalize, source)
                })?
            };
            if !canonical.starts_with(self.root.as_path()) {
                skip_reason = Some(WalkSkipReason::PathEscape);
            } else if is_directory {
                let owned_metadata;
                let metadata = if let Some(metadata) = target_metadata.as_ref() {
                    metadata
                } else {
                    owned_metadata = fs::metadata(&path).map_err(|source| {
                        WalkError::new(&path, depth, WalkOperation::ReadMetadata, source)
                    })?;
                    &owned_metadata
                };
                let info = if depth == 0 {
                    self.root_directory_info
                        .expect("directory identity was requested")
                } else {
                    directory_info(&canonical, metadata).map_err(|source| {
                        WalkError::new(&path, depth, WalkOperation::ReadMetadata, source)
                    })?
                };
                if self.options.same_file_system
                    && self.root_file_system.is_some()
                    && Some(info.file_system) != self.root_file_system
                {
                    skip_reason = Some(WalkSkipReason::FileSystemBoundary);
                }
                if self.options.follow_links {
                    directory_identity = Some(info.identity);
                }
            }
        }

        if is_directory && skip_reason.is_none() {
            if self
                .options
                .max_depth
                .is_some_and(|max_depth| depth >= max_depth)
            {
                skip_reason = Some(WalkSkipReason::MaxDepth);
            } else if self.options.follow_links
                && directory_identity
                    .as_ref()
                    .is_some_and(|identity| self.active_directories.contains(identity))
                && depth != 0
            {
                skip_reason = Some(WalkSkipReason::SymlinkLoop);
            }
        }

        if is_directory && skip_reason.is_none() {
            self.pending_directory = Some(PendingDirectory {
                path: path.clone(),
                depth,
                identity: directory_identity,
            });
        }

        Ok(WalkEntry {
            root_components: self.root_components,
            path,
            depth,
            is_file,
            is_directory,
            is_symlink,
            bytes,
            skip_reason,
        })
    }

    pub(crate) fn yield_error(&mut self, error: WalkError) -> Result<WalkEntry, WalkError> {
        if self.options.error_policy == ErrorPolicy::Abort {
            self.finished = true;
            self.frames.clear();
            self.open_handles = 0;
            self.pending_directory = None;
            self.active_directories.clear();
        }
        Err(error)
    }
}

fn entry_file_type(
    path: &Path,
    depth: usize,
    file_type: Option<FileType>,
) -> Result<FileType, WalkError> {
    file_type.map_or_else(
        || {
            fs::symlink_metadata(path)
                .map(|metadata| metadata.file_type())
                .map_err(|source| WalkError::new(path, depth, WalkOperation::ReadMetadata, source))
        },
        Ok,
    )
}
