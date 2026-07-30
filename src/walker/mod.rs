use crate::report::{FileIdentity, FileVersion};
use crate::walk_builder::{EntryFilter, EntrySorter};
use crate::walk_platform::{PlatformDirectoryInfo, directory_info};
use crate::walk_types::{DirectoryIdentity, FileSystemId};
pub use crate::walk_types::{
    ErrorPolicy, RootSymlinkPolicy, WalkEntry, WalkError, WalkOperation, WalkOptions,
    WalkSkipReason,
};
use std::collections::{HashSet, VecDeque};
use std::fs::{self, FileType};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

mod construction;

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
    #[allow(clippy::inline_always)]
    #[inline(always)]
    pub(crate) fn next(&mut self) -> Option<io::Result<fs::DirEntry>> {
        match self {
            Self::Open(entries) => entries.next(),
            Self::Buffered(entries) => entries.pop_front(),
        }
    }

    #[allow(clippy::inline_always)]
    #[inline(always)]
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
    pub(crate) root_bytes: Option<u64>,
    pub(crate) root_version: Option<FileVersion>,
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
    pub(crate) skip_stdout: Option<FileIdentity>,
    pub(crate) contents_first: bool,
    pub(crate) deferred_entry: Option<WalkEntry>,
    pub(crate) plain_entries: bool,
}
