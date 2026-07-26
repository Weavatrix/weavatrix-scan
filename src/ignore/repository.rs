use super::git::{gitconfig_excludes_path, resolve_git_directory};
use super::overrides::OverrideRules;
use super::repository_source::find_repository_root;
use super::{
    IgnoreRules, RepositoryMatch, RuleAction, SourceRank, match_rules, normalized_evidence_location,
};
use crate::config::{IgnorePolicy, ScanOptions};
use crate::error::{Error, Result};
use crate::hidden::is_hidden;
use crate::path::normalized_relative_path;
use crate::report::{IgnoreSourceEvidence, IgnoreSourceKind, ScanWarning};
use std::borrow::Cow;
use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};

/// A reusable, lazily cached repository ignore matcher.
///
/// Paths may be absolute or relative to the configured scan root. Directory
/// rule files are loaded once and cached, which makes repeated incremental
/// checks cheap without walking the repository again.
#[derive(Debug, Clone)]
pub struct RepositoryMatcher {
    pub(super) scan_root: PathBuf,
    pub(super) options: ScanOptions,
    pub(super) match_root: PathBuf,
    pub(super) scan_base: String,
    pub(super) ignore_files: Vec<String>,
    pub(super) case_insensitive: bool,
    pub(super) error_policy: crate::walker::ErrorPolicy,
    pub(super) overrides: OverrideRules,
    pub(super) skip_hidden: bool,
    pub(super) base_rules: IgnoreRules,
    pub(super) directories: HashMap<PathBuf, IgnoreRules>,
    pub(super) sources: Vec<IgnoreSourceEvidence>,
    pub(super) warnings: Vec<ScanWarning>,
    pub(super) portable: bool,
}

impl RepositoryMatcher {
    /// Builds a matcher with the scanner's reproducible default policy.
    ///
    /// # Errors
    ///
    /// Returns an error when the root cannot be resolved or inspected.
    pub fn new(root: impl AsRef<Path>) -> Result<Self> {
        Self::with_options(root, &ScanOptions::default())
    }

    /// Builds a matcher from scanner selection options.
    ///
    /// # Errors
    ///
    /// Returns an error for an invalid root or, under `ErrorPolicy::Abort`,
    /// when a configured ignore source cannot be read.
    pub fn with_options(root: impl AsRef<Path>, options: &ScanOptions) -> Result<Self> {
        let requested = root.as_ref();
        let scan_root = requested
            .canonicalize()
            .map_err(|source| Error::io(requested, source))?;
        if !scan_root.is_dir() {
            return Err(Error::InvalidRoot(scan_root));
        }
        let repository_root = find_repository_root(&scan_root);
        let git_sources_enabled = !options.ignore_policy.require_git || repository_root.is_some();
        let match_root = if options.ignore_policy.parent_rules {
            repository_root.clone().unwrap_or_else(|| scan_root.clone())
        } else {
            scan_root.clone()
        };
        let (overrides, override_errors, override_evidence) =
            OverrideRules::new(&options.override_rules, options.ignore_case_insensitive);
        let mut matcher = Self {
            scan_base: scan_root
                .strip_prefix(&match_root)
                .map_or_else(|_| String::new(), normalized_relative_path),
            scan_root,
            options: options.clone(),
            match_root,
            ignore_files: options
                .ignore_files
                .iter()
                .filter(|name| source_enabled(name, &options.ignore_policy, git_sources_enabled))
                .cloned()
                .collect(),
            case_insensitive: options.ignore_case_insensitive,
            error_policy: options.walk.error_policy,
            overrides,
            skip_hidden: options.skip_hidden,
            base_rules: IgnoreRules::default(),
            directories: HashMap::new(),
            sources: override_evidence.into_iter().collect(),
            warnings: Vec::new(),
            portable: true,
        };
        matcher.handle_errors(override_errors)?;
        matcher.load_configured_sources(
            options,
            repository_root.as_deref(),
            git_sources_enabled,
        )?;
        let root = matcher.match_root.clone();
        matcher.prepare_directory_inner(&root)?;
        if matcher.scan_root != matcher.match_root {
            let scan_root = matcher.scan_root.clone();
            matcher.prepare_directory_inner(&scan_root)?;
        }
        Ok(matcher)
    }

    fn load_configured_sources(
        &mut self,
        options: &ScanOptions,
        repository_root: Option<&Path>,
        git_sources_enabled: bool,
    ) -> Result<()> {
        if git_sources_enabled
            && options.ignore_policy.git_global
            && let Some(path) = gitconfig_excludes_path(repository_root)
        {
            self.load_static_source(
                &path,
                "",
                SourceRank::GitGlobal,
                IgnoreSourceKind::GitGlobal,
                "<git-global>",
            )?;
            self.portable &= !path.is_file();
        }
        if git_sources_enabled
            && options.ignore_policy.git_exclude
            && let Some(git_directory) = repository_root.and_then(resolve_git_directory)
        {
            let path = git_directory.join("info").join("exclude");
            self.load_static_source(
                &path,
                "",
                SourceRank::GitExclude,
                IgnoreSourceKind::GitExclude,
                ".git/info/exclude",
            )?;
            self.portable &= !path.is_file();
        }
        for path in &options.ignore_policy.explicit_files {
            self.load_explicit_source(path)?;
        }
        Ok(())
    }

    fn load_explicit_source(&mut self, path: &Path) -> Result<()> {
        let configured = if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.scan_root.join(path)
        };
        let absolute = configured.canonicalize().unwrap_or(configured);
        let location = normalized_evidence_location(&absolute, &self.match_root);
        let base = self
            .scan_root
            .strip_prefix(&self.match_root)
            .map_or_else(|_| String::new(), normalized_relative_path);
        self.load_static_source(
            &absolute,
            &base,
            SourceRank::Explicit,
            IgnoreSourceKind::Explicit,
            &location,
        )?;
        if absolute.is_file() && !absolute.starts_with(&self.match_root) {
            self.portable = false;
        }
        Ok(())
    }

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
        let rules = self.directories.get(parent).unwrap_or(&self.base_rules);
        match match_rules(&candidate, is_directory, rules) {
            Some(RuleAction::Ignore) => RepositoryMatch::Ignore,
            Some(RuleAction::Include) => RepositoryMatch::Include,
            None if self.skip_hidden && is_hidden(absolute) => RepositoryMatch::Hidden,
            None => RepositoryMatch::None,
        }
    }
}

fn source_enabled(name: &str, policy: &IgnorePolicy, git_sources_enabled: bool) -> bool {
    match name {
        ".gitignore" => policy.git_ignore && git_sources_enabled,
        ".ignore" => policy.dot_ignore,
        _ => policy.custom_ignore,
    }
}
