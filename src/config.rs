use crate::walker::{ErrorPolicy, WalkOptions};
use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::path::Path;

const DEFAULT_MAX_FILE_BYTES: u64 = 1_500_000;
const DEFAULT_IGNORE_FILES: &[&str] = &[".gitignore", ".weavatrixignore"];
const DEFAULT_SKIP_DIRECTORIES: &[&str] = &[
    ".git",
    ".hg",
    ".svn",
    ".venv",
    "__pycache__",
    "build",
    "coverage",
    "dist",
    "node_modules",
    "target",
    "vendor",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StandardSkips {
    Enabled,
    Disabled,
}

#[derive(Debug, Clone)]
pub struct ScanOptions {
    pub max_file_bytes: u64,
    pub extensions: BTreeSet<String>,
    pub ignore_files: Vec<String>,
    /// Match ignore patterns without ASCII case sensitivity.
    pub ignore_case_insensitive: bool,
    pub standard_skips: StandardSkips,
    pub hash_file_contents: bool,
    pub detect_binary_files: bool,
    /// Content-inspection workers. Zero selects the available parallelism.
    pub parallelism: usize,
    /// Low-level traversal policy.
    pub walk: WalkOptions,
}

impl Default for ScanOptions {
    fn default() -> Self {
        Self {
            max_file_bytes: DEFAULT_MAX_FILE_BYTES,
            extensions: BTreeSet::new(),
            ignore_files: DEFAULT_IGNORE_FILES
                .iter()
                .map(ToString::to_string)
                .collect(),
            ignore_case_insensitive: false,
            standard_skips: StandardSkips::Enabled,
            hash_file_contents: true,
            detect_binary_files: true,
            parallelism: 0,
            walk: WalkOptions::default().with_metadata(true),
        }
    }
}

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

    #[must_use]
    pub const fn with_ignore_case_insensitive(mut self, enabled: bool) -> Self {
        self.ignore_case_insensitive = enabled;
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

    /// Sets content-inspection workers. Zero restores automatic selection.
    #[must_use]
    pub const fn with_parallelism(mut self, parallelism: usize) -> Self {
        self.parallelism = parallelism;
        self
    }

    #[must_use]
    pub const fn with_max_depth(mut self, max_depth: Option<usize>) -> Self {
        self.walk.max_depth = max_depth;
        self
    }

    #[must_use]
    pub const fn with_max_open(mut self, max_open: usize) -> Self {
        self.walk.max_open = if max_open == 0 { 1 } else { max_open };
        self
    }

    #[must_use]
    pub const fn with_same_file_system(mut self, enabled: bool) -> Self {
        self.walk.same_file_system = enabled;
        self
    }

    #[must_use]
    pub const fn with_follow_links(mut self, enabled: bool) -> Self {
        self.walk.follow_links = enabled;
        self
    }

    #[must_use]
    pub const fn with_error_policy(mut self, policy: ErrorPolicy) -> Self {
        self.walk.error_policy = policy;
        self
    }

    pub(crate) fn should_skip_directory(&self, name: &OsStr) -> bool {
        self.standard_skips == StandardSkips::Enabled
            && DEFAULT_SKIP_DIRECTORIES
                .iter()
                .any(|candidate| name == OsStr::new(candidate))
    }

    pub(crate) fn accepts_extension(&self, path: &Path) -> bool {
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

    pub(crate) fn worker_count(&self, file_count: usize) -> usize {
        if file_count == 0 {
            return 1;
        }
        let requested = if self.parallelism == 0 {
            std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get)
        } else {
            self.parallelism
        };
        requested.min(file_count.div_ceil(128)).max(1)
    }

    pub(crate) const fn walk_options(&self) -> WalkOptions {
        self.walk
    }
}
