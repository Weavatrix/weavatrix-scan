use super::{
    CacheValidationPolicy, CancellationToken, ContentDiscoveryMode, ContentValidationPolicy,
    DEFAULT_SKIP_DIRECTORIES, Duration, EvidenceMode, FileTypeMatch, IgnorePolicy, NamedFileTypes,
    OsStr, Path, ScanOptions, StandardSkips,
};

impl ScanOptions {
    #[must_use]
    pub fn with_extensions<I, S>(mut self, extensions: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        self.extensions = extensions
            .into_iter()
            .map(|item| item.as_ref().trim_start_matches('.').to_ascii_lowercase())
            .collect();
        self
    }

    /// Replaces named file-type definitions and selections.
    #[must_use]
    pub fn with_file_types(mut self, file_types: NamedFileTypes) -> Self {
        self.file_types = file_types;
        self
    }

    #[must_use]
    pub fn with_ignore_files<I, S>(mut self, names: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        self.ignore_files = names
            .into_iter()
            .map(|item| item.as_ref().to_owned())
            .collect();
        self
    }

    /// Replaces request-level override globs.
    ///
    /// Like `ignore::Override`, ordinary patterns include matching paths and
    /// leading `!` patterns exclude them.
    #[must_use]
    pub fn with_override_rules<I, S>(mut self, rules: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        self.override_rules = rules
            .into_iter()
            .map(|item| item.as_ref().to_owned())
            .collect();
        self
    }

    #[must_use]
    pub const fn with_ignore_case_insensitive(mut self, enabled: bool) -> Self {
        self.ignore_case_insensitive = enabled;
        self
    }

    #[must_use]
    pub fn with_ignore_policy(mut self, policy: IgnorePolicy) -> Self {
        self.ignore_policy = policy;
        self
    }

    #[must_use]
    pub const fn with_skip_hidden(mut self, enabled: bool) -> Self {
        self.skip_hidden = enabled;
        self
    }

    /// Disables file-content reads for the fastest metadata-only discovery.
    ///
    /// The resulting report does not contain content hashes and may include
    /// binary files whose extension matches the configured filter.
    #[must_use]
    pub fn metadata_only(mut self) -> Self {
        self.hash_file_contents = false;
        self.detect_binary_files = false;
        self
    }

    /// Keeps only the selected manifest and warnings, without skip evidence.
    #[must_use]
    pub const fn selected_files_only(mut self) -> Self {
        self.evidence = EvidenceMode::SelectedFiles;
        self
    }

    /// Sets the shared traversal and content worker default.
    ///
    /// A later traversal- or content-specific override takes precedence.
    #[must_use]
    pub const fn with_parallelism(mut self, parallelism: usize) -> Self {
        self.parallelism = parallelism;
        self
    }

    /// Sets traversal workers without changing content-inspection workers.
    #[must_use]
    pub const fn with_traversal_parallelism(mut self, parallelism: usize) -> Self {
        self.traversal_parallelism = Some(parallelism);
        self
    }

    /// Sets content-inspection workers without changing traversal workers.
    #[must_use]
    pub const fn with_content_parallelism(mut self, parallelism: usize) -> Self {
        self.content_parallelism = Some(parallelism);
        self
    }

    /// Sets candidate discovery for content visits.
    #[must_use]
    pub const fn with_content_discovery(mut self, mode: ContentDiscoveryMode) -> Self {
        self.content_discovery = mode;
        self
    }

    #[must_use]
    pub const fn with_max_entries(mut self, max_entries: Option<u64>) -> Self {
        self.limits.max_entries = max_entries;
        self
    }

    #[must_use]
    pub const fn with_max_total_bytes(mut self, max_total_bytes: Option<u64>) -> Self {
        self.limits.max_total_bytes = max_total_bytes;
        self
    }

    #[must_use]
    pub const fn with_timeout(mut self, timeout: Option<Duration>) -> Self {
        self.limits.timeout = timeout;
        self
    }

    #[must_use]
    pub fn with_cancellation(mut self, cancellation: CancellationToken) -> Self {
        self.cancellation = Some(cancellation);
        self
    }

    #[must_use]
    pub const fn with_cache_validation(mut self, policy: CacheValidationPolicy) -> Self {
        self.cache_validation = policy;
        self
    }

    #[must_use]
    pub const fn with_content_validation(mut self, policy: ContentValidationPolicy) -> Self {
        self.content_validation = policy;
        self
    }

    pub(crate) fn should_skip_directory(&self, name: &OsStr) -> bool {
        self.standard_skips == StandardSkips::Enabled
            && DEFAULT_SKIP_DIRECTORIES
                .iter()
                .any(|candidate| name == OsStr::new(candidate))
    }

    pub(crate) fn accepts_extension(&self, path: &Path, relative: &str) -> bool {
        if self.extensions.is_empty() && !self.file_types.is_active() {
            return true;
        }
        match self.file_types.matched(path, relative) {
            FileTypeMatch::Include => return true,
            FileTypeMatch::Exclude => return false,
            FileTypeMatch::None => {}
        }
        if self.extensions.is_empty() && self.file_types.has_includes() {
            return false;
        }
        if self.extensions.is_empty() {
            return true;
        }
        let Some(extension) = path.extension().and_then(|value| value.to_str()) else {
            return false;
        };
        self.contains_extension(extension)
            || (extension.bytes().any(|byte| byte.is_ascii_uppercase())
                && self.contains_extension(&extension.to_ascii_lowercase()))
    }

    fn contains_extension(&self, extension: &str) -> bool {
        if self.extensions.len() <= 8 {
            self.extensions
                .iter()
                .any(|candidate| candidate == extension)
        } else {
            self.extensions.contains(extension)
        }
    }
}
