use crate::ignore::{RepositoryMatch, RepositoryMatcher};
use crate::scan_match::skip_kind_for_match;
use crate::walk_platform::{FileSystemId, directory_info};
use crate::{
    Error, IgnoreSourceEvidence, Result, ScanOptions, ScanWarning, SkipKind, WalkEntry,
    WalkSkipReason,
};
use std::fs;
use std::path::{Component, Path, PathBuf};

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

impl SelectionMatcher {
    /// Builds a matcher with [`ScanOptions::default`].
    ///
    /// # Errors
    ///
    /// Returns an error when the root or required ignore configuration cannot
    /// be read.
    pub fn new(root: impl AsRef<Path>) -> Result<Self> {
        Self::with_options(root, &ScanOptions::default())
    }

    /// Builds a matcher from the same options used by [`crate::Scanner`].
    ///
    /// # Errors
    ///
    /// Returns an error when the root or required ignore configuration cannot
    /// be read.
    pub fn with_options(root: impl AsRef<Path>, options: &ScanOptions) -> Result<Self> {
        let repository = RepositoryMatcher::with_options(root, options)?;
        let root_file_system = if options.walk.same_file_system {
            let metadata = fs::metadata(repository.root())
                .map_err(|source| Error::io(repository.root(), source))?;
            Some(
                directory_info(repository.root(), &metadata)
                    .map_err(|source| Error::io(repository.root(), source))?
                    .file_system,
            )
        } else {
            None
        };
        Ok(Self {
            repository,
            options: options.clone(),
            root_file_system,
        })
    }

    /// Applies the complete selection policy to one existing path.
    ///
    /// This convenience method performs `symlink_metadata` and, when links are
    /// followed, target metadata. Use [`Self::matched_entry`] inside an existing
    /// Weavatrix walk to avoid those extra reads.
    ///
    /// # Errors
    ///
    /// Returns an error for escaped paths, missing entries, metadata failures,
    /// or required ignore-source failures under [`crate::ErrorPolicy::Abort`].
    pub fn matched(&mut self, path: impl AsRef<Path>) -> Result<SelectionDecision> {
        let relative = self.repository.normalize(path)?;
        let absolute = self.repository.root().join(&relative);
        let depth = relative_depth(&relative);
        if let Some(decision) = self.match_ancestors(&relative)? {
            return Ok(decision);
        }
        let link_metadata =
            fs::symlink_metadata(&absolute).map_err(|source| Error::io(&absolute, source))?;
        let is_symlink = link_metadata.file_type().is_symlink();
        if is_symlink && !self.options.walk.follow_links {
            return Ok(SelectionDecision::skipped(
                SkipKind::Symlink,
                RepositoryMatch::None,
            ));
        }
        let metadata = if is_symlink {
            let canonical = absolute
                .canonicalize()
                .map_err(|source| Error::io(&absolute, source))?;
            if !canonical.starts_with(self.repository.root()) {
                return Ok(SelectionDecision::skipped(
                    SkipKind::PathEscape,
                    RepositoryMatch::None,
                ));
            }
            fs::metadata(&absolute).map_err(|source| Error::io(&absolute, source))?
        } else {
            link_metadata
        };
        if metadata.is_dir()
            && let Some(decision) = self.file_system_decision(&absolute, &metadata)?
        {
            return Ok(decision);
        }
        self.classify(
            &absolute,
            &relative,
            depth,
            metadata.is_file(),
            metadata.is_dir(),
            false,
            metadata.len(),
            None,
        )
    }

    /// Applies the complete selection policy to a walker entry.
    ///
    /// # Errors
    ///
    /// Returns an error for escaped paths, metadata failures when the walker
    /// did not collect file metadata, or required ignore-source failures.
    pub fn matched_entry(&mut self, entry: &WalkEntry) -> Result<SelectionDecision> {
        let relative = self.repository.normalize(entry.relative_path())?;
        let absolute = self.repository.root().join(&relative);
        let bytes = if entry.is_file() {
            match entry.bytes() {
                Some(bytes) => bytes,
                None => fs::metadata(&absolute)
                    .map_err(|source| Error::io(&absolute, source))?
                    .len(),
            }
        } else {
            0
        };
        self.classify(
            &absolute,
            &relative,
            entry.depth(),
            entry.is_file(),
            entry.is_dir(),
            entry.is_symlink(),
            bytes,
            entry.skip_reason(),
        )
    }

    /// Atomically reloads ignore inputs while retaining the full selection
    /// options.
    ///
    /// # Errors
    ///
    /// Returns an error when replacement matcher construction fails.
    pub fn refresh(&mut self) -> Result<bool> {
        self.repository.refresh()
    }

    /// Returns the canonical matcher root.
    #[must_use]
    pub fn root(&self) -> &Path {
        self.repository.root()
    }

    /// Returns the immutable selection options snapshot.
    #[must_use]
    pub const fn options(&self) -> &ScanOptions {
        &self.options
    }

    /// Returns evidence for every loaded ignore source.
    #[must_use]
    pub fn sources(&self) -> &[IgnoreSourceEvidence] {
        self.repository.sources()
    }

    /// Returns non-fatal matcher diagnostics.
    #[must_use]
    pub fn warnings(&self) -> &[ScanWarning] {
        self.repository.warnings()
    }

    /// Returns whether all matcher inputs are portable.
    #[must_use]
    pub fn portable(&self) -> bool {
        self.repository.portable()
    }

    #[allow(clippy::too_many_arguments)]
    fn classify(
        &mut self,
        absolute: &Path,
        relative: &Path,
        depth: usize,
        is_file: bool,
        is_directory: bool,
        is_symlink: bool,
        bytes: u64,
        walk_skip: Option<WalkSkipReason>,
    ) -> Result<SelectionDecision> {
        if depth == 0 {
            if let Some(reason) = walk_skip {
                return Ok(SelectionDecision::skipped(
                    skip_kind(reason),
                    RepositoryMatch::None,
                ));
            }
            if is_symlink && !self.options.walk.follow_links {
                return Ok(SelectionDecision::skipped(
                    SkipKind::Symlink,
                    RepositoryMatch::None,
                ));
            }
            if is_directory {
                self.repository.prepare_directory(absolute)?;
                return Ok(SelectionDecision::directory(RepositoryMatch::None));
            }
            if is_file {
                return self.classify_file(absolute, relative, bytes);
            }
            return Ok(SelectionDecision::unselected());
        }
        if depth < self.options.effective_min_depth() && !is_directory {
            return Ok(SelectionDecision::unselected());
        }
        if is_symlink && !self.options.walk.follow_links {
            return Ok(SelectionDecision::skipped(
                SkipKind::Symlink,
                RepositoryMatch::None,
            ));
        }
        if let Some(reason) = walk_skip {
            return Ok(SelectionDecision::skipped(
                skip_kind(reason),
                RepositoryMatch::None,
            ));
        }
        if self
            .options
            .walk
            .max_depth
            .is_some_and(|maximum| depth > maximum || (is_directory && depth == maximum))
        {
            return Ok(SelectionDecision::skipped(
                SkipKind::MaxDepth,
                RepositoryMatch::None,
            ));
        }
        if is_directory {
            let matched = self.repository.matched(absolute, true)?;
            if let Some(kind) = skip_kind_for_match(matched) {
                return Ok(SelectionDecision::skipped(kind, matched));
            }
            if matched != RepositoryMatch::OverrideInclude
                && self
                    .options
                    .should_skip_directory(absolute.file_name().unwrap_or(absolute.as_os_str()))
            {
                return Ok(SelectionDecision::skipped(
                    SkipKind::StandardDirectory,
                    matched,
                ));
            }
            self.repository.prepare_directory(absolute)?;
            return Ok(SelectionDecision::directory(matched));
        }
        if is_file {
            return self.classify_file(absolute, relative, bytes);
        }
        Ok(SelectionDecision::unselected())
    }

    fn classify_file(
        &mut self,
        absolute: &Path,
        relative: &Path,
        bytes: u64,
    ) -> Result<SelectionDecision> {
        let matched = self.repository.matched(absolute, false)?;
        if let Some(kind) = skip_kind_for_match(matched) {
            return Ok(SelectionDecision::skipped(kind, matched));
        }
        let normalized = crate::path::normalized_relative_path(relative);
        if matched != RepositoryMatch::OverrideInclude
            && !self.options.accepts_extension(absolute, &normalized)
        {
            return Ok(SelectionDecision::skipped(SkipKind::Extension, matched));
        }
        if bytes > self.options.max_file_bytes {
            return Ok(SelectionDecision::skipped(SkipKind::Oversized, matched));
        }
        Ok(SelectionDecision::selected(matched))
    }

    fn match_ancestors(&mut self, relative: &Path) -> Result<Option<SelectionDecision>> {
        let Some(parent) = relative.parent() else {
            return Ok(None);
        };
        let mut current = PathBuf::new();
        for component in parent.components() {
            let Component::Normal(name) = component else {
                continue;
            };
            current.push(name);
            let depth = relative_depth(&current);
            if self
                .options
                .walk
                .max_depth
                .is_some_and(|maximum| depth >= maximum)
            {
                return Ok(Some(SelectionDecision::skipped(
                    SkipKind::MaxDepth,
                    RepositoryMatch::None,
                )));
            }
            let absolute = self.repository.root().join(&current);
            let link_metadata =
                fs::symlink_metadata(&absolute).map_err(|source| Error::io(&absolute, source))?;
            let is_symlink = link_metadata.file_type().is_symlink();
            if is_symlink && !self.options.walk.follow_links {
                return Ok(Some(SelectionDecision::skipped(
                    SkipKind::Symlink,
                    RepositoryMatch::None,
                )));
            }
            let metadata = if is_symlink {
                let canonical = absolute
                    .canonicalize()
                    .map_err(|source| Error::io(&absolute, source))?;
                if !canonical.starts_with(self.repository.root()) {
                    return Ok(Some(SelectionDecision::skipped(
                        SkipKind::PathEscape,
                        RepositoryMatch::None,
                    )));
                }
                fs::metadata(&absolute).map_err(|source| Error::io(&absolute, source))?
            } else {
                link_metadata
            };
            if !metadata.is_dir() {
                return Ok(Some(SelectionDecision::unselected()));
            }
            if let Some(decision) = self.file_system_decision(&absolute, &metadata)? {
                return Ok(Some(decision));
            }
            let matched = self.repository.matched(&absolute, true)?;
            if let Some(kind) = skip_kind_for_match(matched) {
                return Ok(Some(SelectionDecision::skipped(kind, matched)));
            }
            if matched != RepositoryMatch::OverrideInclude
                && self.options.should_skip_directory(name)
            {
                return Ok(Some(SelectionDecision::skipped(
                    SkipKind::StandardDirectory,
                    matched,
                )));
            }
            self.repository.prepare_directory(&absolute)?;
        }
        Ok(None)
    }

    fn file_system_decision(
        &self,
        absolute: &Path,
        metadata: &fs::Metadata,
    ) -> Result<Option<SelectionDecision>> {
        let Some(root_file_system) = self.root_file_system else {
            return Ok(None);
        };
        let current = directory_info(absolute, metadata)
            .map_err(|source| Error::io(absolute, source))?
            .file_system;
        Ok((current != root_file_system).then(|| {
            SelectionDecision::skipped(SkipKind::FileSystemBoundary, RepositoryMatch::None)
        }))
    }
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
