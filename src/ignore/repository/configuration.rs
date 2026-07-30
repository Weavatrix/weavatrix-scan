use super::{
    Error, HashMap, IgnoreRules, IgnoreSourceKind, OverrideRules, Path, RepositoryMatcher, Result,
    ScanOptions, SourceRank, find_repository_root, gitconfig_excludes_path,
    normalized_evidence_location, normalized_relative_path, resolve_git_directory, source_enabled,
};

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
}
