use crate::config::ScanOptions;
use crate::error::{Error, Result};
use crate::ignore::{RepositoryMatch, RepositoryMatcher};
use crate::report::{IgnoreSourceEvidence, ScanWarning, SkipKind};
use crate::scan_match::skip_kind_for_match;
use crate::walk_platform::directory_info;
use crate::walk_types::{FileSystemId, WalkEntry, WalkSkipReason};
use std::fs;
use std::path::{Component, Path, PathBuf};

mod classification;
mod construction;
mod queries;

/// Final selection outcome for one filesystem entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum SelectionDisposition {
    /// A regular file accepted by every configured selection filter.
    SelectedFile,
    /// A directory accepted for traversal.
    TraverseDirectory,
    /// An entry excluded by a typed scanner policy.
    Skipped(SkipKind),
    /// An entry suppressed without a typed skip, such as a file below
    /// `min_depth` or a non-file filesystem object.
    Unselected,
}

/// A complete, typed selection result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SelectionDecision {
    disposition: SelectionDisposition,
    repository_match: RepositoryMatch,
}

impl SelectionDecision {
    /// Returns the final entry disposition.
    #[must_use]
    pub const fn disposition(self) -> SelectionDisposition {
        self.disposition
    }

    /// Returns the winning repository/override decision, when it was reached.
    #[must_use]
    pub const fn repository_match(self) -> RepositoryMatch {
        self.repository_match
    }

    /// Returns whether this entry is a selected regular file.
    #[must_use]
    pub const fn is_selected(self) -> bool {
        matches!(self.disposition, SelectionDisposition::SelectedFile)
    }

    /// Returns whether traversal should descend into this directory.
    #[must_use]
    pub const fn should_descend(self) -> bool {
        matches!(self.disposition, SelectionDisposition::TraverseDirectory)
    }

    /// Returns the typed exclusion reason, if the entry was skipped.
    #[must_use]
    pub const fn skip_kind(self) -> Option<SkipKind> {
        match self.disposition {
            SelectionDisposition::Skipped(kind) => Some(kind),
            SelectionDisposition::SelectedFile
            | SelectionDisposition::TraverseDirectory
            | SelectionDisposition::Unselected => None,
        }
    }

    const fn selected(repository_match: RepositoryMatch) -> Self {
        Self {
            disposition: SelectionDisposition::SelectedFile,
            repository_match,
        }
    }

    const fn directory(repository_match: RepositoryMatch) -> Self {
        Self {
            disposition: SelectionDisposition::TraverseDirectory,
            repository_match,
        }
    }

    const fn skipped(kind: SkipKind, repository_match: RepositoryMatch) -> Self {
        Self {
            disposition: SelectionDisposition::Skipped(kind),
            repository_match,
        }
    }

    const fn unselected() -> Self {
        Self {
            disposition: SelectionDisposition::Unselected,
            repository_match: RepositoryMatch::None,
        }
    }
}

/// Reusable matcher for the complete scanner selection policy.
///
/// Unlike [`RepositoryMatcher`], this applies depth, symlink, standard
/// directory, named file-type, extension, and maximum-size policies in
/// addition to hierarchical ignore and override rules. [`Self::matched`]
/// performs the metadata read needed to classify a standalone path;
/// [`Self::matched_entry`] reuses metadata already captured by a [`WalkEntry`].
///
/// Stateful traversal-only conditions such as symlink-cycle ancestry are
/// reported by [`WalkEntry::skip_reason`] when `matched_entry` is used.
#[derive(Debug, Clone)]
pub struct SelectionMatcher {
    repository: RepositoryMatcher,
    options: ScanOptions,
    root_file_system: Option<FileSystemId>,
}

fn relative_depth(path: &Path) -> usize {
    path.components()
        .filter(|component| matches!(component, Component::Normal(_)))
        .count()
}

const fn skip_kind(reason: WalkSkipReason) -> SkipKind {
    match reason {
        WalkSkipReason::MaxDepth => SkipKind::MaxDepth,
        WalkSkipReason::FileSystemBoundary => SkipKind::FileSystemBoundary,
        WalkSkipReason::PathEscape => SkipKind::PathEscape,
        WalkSkipReason::SymlinkLoop => SkipKind::SymlinkLoop,
    }
}
