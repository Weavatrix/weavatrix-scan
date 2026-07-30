use super::{
    Error, IgnoreSourceEvidence, Path, RepositoryMatch, Result, ScanOptions, ScanWarning,
    SelectionDecision, SelectionMatcher, SkipKind, WalkEntry, fs, relative_depth,
};

impl SelectionMatcher {
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
}
