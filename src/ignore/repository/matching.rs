use super::{
    Cow, Error, IgnoreRules, IgnoreSourceEvidence, Path, PathBuf, RepositoryMatch,
    RepositoryMatcher, Result, RuleAction, ScanWarning, io, is_hidden_with_hint,
    match_prepared_rules, match_rules, normalized_relative_path,
};

impl RepositoryMatcher {
    /// Returns the winning typed decision from overrides, ignore rules, and
    /// the optional hidden-file filter.
    ///
    /// # Errors
    ///
    /// Returns an error when the path escapes the matcher root or a required
    /// rule file cannot be read under `ErrorPolicy::Abort`.
    pub fn matched(
        &mut self,
        path: impl AsRef<Path>,
        is_directory: bool,
    ) -> Result<RepositoryMatch> {
        let absolute = self.absolute_path(path.as_ref())?;
        self.ensure_scan_scope(&absolute)?;
        let parent = absolute.parent().unwrap_or(&self.match_root).to_path_buf();
        self.prepare_directory_inner(&parent)?;
        let scan_relative = absolute.strip_prefix(&self.scan_root).map_err(|_| {
            Error::io(
                &absolute,
                io::Error::new(io::ErrorKind::InvalidInput, "path escapes matcher root"),
            )
        })?;
        Ok(self.matched_prepared(
            &normalized_relative_path(scan_relative),
            &parent,
            &absolute,
            is_directory,
            None,
            false,
        ))
    }

    /// Returns whether a path is excluded by the effective selection policy.
    ///
    /// # Errors
    ///
    /// Returns an error under the same conditions as [`Self::matched`].
    pub fn is_ignored(&mut self, path: impl AsRef<Path>, is_directory: bool) -> Result<bool> {
        self.matched(path, is_directory)
            .map(RepositoryMatch::is_ignored)
    }

    /// Normalizes a path and returns its lossless root-relative representation.
    ///
    /// Relative inputs are interpreted from this matcher's root. Absolute
    /// inputs outside the root and paths containing parent traversal are
    /// rejected.
    ///
    /// # Errors
    ///
    /// Returns an error when the path escapes the configured scan root.
    pub fn normalize(&self, path: impl AsRef<Path>) -> Result<PathBuf> {
        let absolute = self.absolute_path(path.as_ref())?;
        self.ensure_scan_scope(&absolute)?;
        absolute
            .strip_prefix(&self.scan_root)
            .map(Path::to_path_buf)
            .map_err(|_| {
                Error::io(
                    &absolute,
                    io::Error::new(io::ErrorKind::InvalidInput, "path escapes matcher root"),
                )
            })
    }

    /// Preloads a directory's rules for subsequent child checks.
    ///
    /// # Errors
    ///
    /// Returns an error under the same conditions as [`Self::is_ignored`].
    pub fn prepare_directory(&mut self, directory: impl AsRef<Path>) -> Result<()> {
        let absolute = self.absolute_path(directory.as_ref())?;
        self.ensure_scan_scope(&absolute)?;
        self.prepare_directory_inner(&absolute)
    }

    /// Reloads ignore inputs while preserving the configured matcher policy.
    ///
    /// Returns true when effective selection inputs changed.
    ///
    /// # Errors
    ///
    /// Returns an error under the same conditions as [`Self::with_options`].
    pub fn refresh(&mut self) -> Result<bool> {
        let refreshed = Self::with_options(&self.scan_root, &self.options)?;
        let changed = self.sources != refreshed.sources
            || self.portable != refreshed.portable
            || self.warnings != refreshed.warnings;
        *self = refreshed;
        Ok(changed)
    }

    #[must_use]
    pub fn root(&self) -> &Path {
        &self.scan_root
    }

    #[must_use]
    pub fn sources(&self) -> &[IgnoreSourceEvidence] {
        &self.sources
    }

    #[must_use]
    pub fn warnings(&self) -> &[ScanWarning] {
        &self.warnings
    }

    #[must_use]
    pub const fn portable(&self) -> bool {
        self.portable
    }

    pub(crate) fn matched_prepared(
        &self,
        scan_relative: &str,
        parent: &Path,
        absolute: &Path,
        is_directory: bool,
        hidden: Option<bool>,
        ancestors_prepared: bool,
    ) -> RepositoryMatch {
        let rules = self.directories.get(parent).unwrap_or(&self.base_rules);
        self.matched_with_rules(
            scan_relative,
            absolute,
            is_directory,
            hidden,
            rules,
            ancestors_prepared,
        )
    }

    pub(crate) fn prepared_rules(&self, parent: &Path) -> &IgnoreRules {
        self.directories.get(parent).unwrap_or(&self.base_rules)
    }

    pub(crate) fn matched_with_rules(
        &self,
        scan_relative: &str,
        absolute: &Path,
        is_directory: bool,
        hidden: Option<bool>,
        rules: &IgnoreRules,
        ancestors_prepared: bool,
    ) -> RepositoryMatch {
        let override_match = self.overrides.matched(scan_relative, is_directory);
        if override_match != RepositoryMatch::None {
            return override_match;
        }
        let candidate = if self.scan_base.is_empty() {
            Cow::Borrowed(scan_relative)
        } else {
            Cow::Owned(format!("{}/{scan_relative}", self.scan_base))
        };
        let matched = if ancestors_prepared {
            match_prepared_rules(&candidate, is_directory, rules)
        } else {
            match_rules(&candidate, is_directory, rules)
        };
        match matched {
            Some(RuleAction::Ignore) => RepositoryMatch::Ignore,
            Some(RuleAction::Include) => RepositoryMatch::Include,
            None if self.skip_hidden && is_hidden_with_hint(absolute, hidden) => {
                RepositoryMatch::Hidden
            }
            None => RepositoryMatch::None,
        }
    }
}
