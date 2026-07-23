use super::git::{gitconfig_excludes_path, resolve_git_directory};
use super::repository_source::find_repository_root;
use super::{IgnoreRules, SourceRank, is_ignored, normalized_evidence_location};
use crate::config::ScanOptions;
use crate::error::{Error, Result};
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
#[derive(Debug)]
pub struct RepositoryMatcher {
    pub(super) scan_root: PathBuf,
    pub(super) match_root: PathBuf,
    pub(super) scan_base: String,
    pub(super) ignore_files: Vec<String>,
    pub(super) case_insensitive: bool,
    pub(super) error_policy: crate::walker::ErrorPolicy,
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
        let match_root = if options.ignore_policy.parent_rules {
            repository_root.clone().unwrap_or_else(|| scan_root.clone())
        } else {
            scan_root.clone()
        };
        let mut matcher = Self {
            scan_base: scan_root
                .strip_prefix(&match_root)
                .map_or_else(|_| String::new(), normalized_relative_path),
            scan_root,
            match_root,
            ignore_files: options.ignore_files.clone(),
            case_insensitive: options.ignore_case_insensitive,
            error_policy: options.walk.error_policy,
            base_rules: IgnoreRules::default(),
            directories: HashMap::new(),
            sources: Vec::new(),
            warnings: Vec::new(),
            portable: true,
        };
        matcher.load_configured_sources(options, repository_root.as_deref())?;
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
    ) -> Result<()> {
        if options.ignore_policy.git_global
            && let Some(path) = gitconfig_excludes_path()
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
        if options.ignore_policy.git_exclude
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

    /// Returns whether a path is excluded by the effective ignore policy.
    ///
    /// # Errors
    ///
    /// Returns an error when the path escapes the matcher root or a required
    /// rule file cannot be read under `ErrorPolicy::Abort`.
    pub fn is_ignored(&mut self, path: impl AsRef<Path>, is_directory: bool) -> Result<bool> {
        let absolute = self.absolute_path(path.as_ref())?;
        self.ensure_scan_scope(&absolute)?;
        let parent = absolute.parent().unwrap_or(&self.match_root).to_path_buf();
        self.prepare_directory_inner(&parent)?;
        let relative = absolute.strip_prefix(&self.match_root).map_err(|_| {
            Error::io(
                &absolute,
                io::Error::new(io::ErrorKind::InvalidInput, "path escapes matcher root"),
            )
        })?;
        let relative = normalized_relative_path(relative);
        let rules = self.directories.get(&parent).unwrap_or(&self.base_rules);
        Ok(is_ignored(&relative, is_directory, rules))
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

    pub(crate) fn is_ignored_prepared(
        &self,
        scan_relative: &str,
        parent: &Path,
        is_directory: bool,
    ) -> bool {
        let candidate = if self.scan_base.is_empty() {
            Cow::Borrowed(scan_relative)
        } else {
            Cow::Owned(format!("{}/{scan_relative}", self.scan_base))
        };
        let rules = self.directories.get(parent).unwrap_or(&self.base_rules);
        is_ignored(&candidate, is_directory, rules)
    }
}
