use crate::control::CancellationToken;
use crate::walker::{ErrorPolicy, WalkOptions};
use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::path::Path;
use std::time::Duration;

mod ignore_policy;
mod limits;

pub use ignore_policy::IgnorePolicy;
pub use limits::ScanLimits;

const DEFAULT_MAX_FILE_BYTES: u64 = 1_500_000;
const DEFAULT_IGNORE_FILES: &[&str] = &[".gitignore", ".ignore", ".weavatrixignore"];
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvidenceMode {
    Complete,
    SelectedFiles,
}

#[derive(Debug, Clone)]
#[allow(clippy::struct_excessive_bools)]
pub struct ScanOptions {
    pub max_file_bytes: u64,
    pub extensions: BTreeSet<String>,
    pub ignore_files: Vec<String>,
    /// High-precedence include/exclude globs using `ignore::Override` syntax.
    pub override_rules: Vec<String>,
    /// Controls parent, repository-exclude and global Git ignore sources.
    pub ignore_policy: IgnorePolicy,
    /// Match ignore patterns without ASCII case sensitivity.
    pub ignore_case_insensitive: bool,
    /// Skip dot-prefixed and native Windows-hidden entries unless included.
    pub skip_hidden: bool,
    pub standard_skips: StandardSkips,
    pub hash_file_contents: bool,
    pub detect_binary_files: bool,
    /// Record typed evidence for entries excluded by policy.
    pub evidence: EvidenceMode,
    /// Content-inspection workers. Zero selects the available parallelism.
    pub parallelism: usize,
    /// Whole-scan resource bounds. All limits are disabled by default.
    pub limits: ScanLimits,
    /// Optional cooperative cancellation signal.
    pub cancellation: Option<CancellationToken>,
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
            override_rules: Vec::new(),
            ignore_policy: IgnorePolicy::default(),
            ignore_case_insensitive: false,
            skip_hidden: false,
            standard_skips: StandardSkips::Enabled,
            hash_file_contents: true,
            detect_binary_files: true,
            evidence: EvidenceMode::Complete,
            parallelism: 0,
            limits: ScanLimits::default(),
            cancellation: None,
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

    /// Sets content-inspection workers. Zero restores automatic selection.
    #[must_use]
    pub const fn with_parallelism(mut self, parallelism: usize) -> Self {
        self.parallelism = parallelism;
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
    pub const fn with_max_depth(mut self, max_depth: Option<usize>) -> Self {
        self.walk.max_depth = max_depth;
        self
    }

    #[must_use]
    pub const fn with_min_depth(mut self, min_depth: usize) -> Self {
        self.walk.min_depth = min_depth;
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
        let mut options = self.walk;
        options.min_depth = 0;
        options
    }

    pub(crate) fn effective_min_depth(&self) -> usize {
        self.walk.max_depth.map_or(self.walk.min_depth, |maximum| {
            self.walk.min_depth.min(maximum)
        })
    }
}
