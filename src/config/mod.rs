use crate::control::CancellationToken;
use crate::file_types::{FileTypeMatch, NamedFileTypes};
use crate::walk_types::WalkOptions;
use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::path::Path;
use std::time::Duration;

mod builder;
mod ignore_policy;
mod limits;
mod runtime;
mod walk;

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

/// Controls how persistent content hashes are validated before reuse.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheValidationPolicy {
    /// Trust stable size, timestamp and available native identity evidence.
    Fast,
    /// Read a whole-file 128-bit fingerprint before reusing the prior SHA-256.
    Strict,
}

/// Controls post-read snapshot verification for newly opened content.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentValidationPolicy {
    /// Verify the opened handle against discovery evidence before reading.
    /// This is appropriate for latency-sensitive local search.
    Fast,
    /// Also re-check native file evidence after reading to reject concurrent
    /// same-size modifications.
    Strict,
}

/// Controls how content candidates are discovered before bounded file reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentDiscoveryMode {
    /// Discover candidates serially and overlap discovery with content reads.
    ///
    /// This keeps memory bounded independently of the number of files.
    Streaming,
    /// Discover candidates with the parallel walker, retain their compact path
    /// evidence, then dispatch bounded content reads.
    ///
    /// This minimizes latency on large, warm repositories at the cost of
    /// memory proportional to the number of selected files.
    BufferedParallel,
}

#[derive(Debug, Clone)]
#[allow(clippy::struct_excessive_bools)]
pub struct ScanOptions {
    pub max_file_bytes: u64,
    pub extensions: BTreeSet<String>,
    /// Reusable named file-pattern groups, combined with `extensions`.
    pub file_types: NamedFileTypes,
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
    /// Traversal and content-inspection workers. Zero selects available
    /// parallelism. Retained as the shared backward-compatible default.
    pub parallelism: usize,
    /// Optional traversal-only worker override. `Some(0)` selects available
    /// parallelism independently of content inspection.
    pub traversal_parallelism: Option<usize>,
    /// Optional content-inspection worker override. `Some(0)` selects
    /// available parallelism independently of traversal.
    pub content_parallelism: Option<usize>,
    /// Whole-scan resource bounds. All limits are disabled by default.
    pub limits: ScanLimits,
    /// Optional cooperative cancellation signal.
    pub cancellation: Option<CancellationToken>,
    /// Persistent hash validation policy.
    pub cache_validation: CacheValidationPolicy,
    /// New content-read validation policy.
    pub content_validation: ContentValidationPolicy,
    /// Candidate-discovery policy for content visits.
    pub content_discovery: ContentDiscoveryMode,
    /// Low-level traversal policy.
    pub walk: WalkOptions,
}

impl Default for ScanOptions {
    fn default() -> Self {
        Self {
            max_file_bytes: DEFAULT_MAX_FILE_BYTES,
            extensions: BTreeSet::new(),
            file_types: NamedFileTypes::default(),
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
            traversal_parallelism: None,
            content_parallelism: None,
            limits: ScanLimits::default(),
            cancellation: None,
            cache_validation: CacheValidationPolicy::Fast,
            content_validation: ContentValidationPolicy::Strict,
            content_discovery: ContentDiscoveryMode::Streaming,
            walk: WalkOptions::default().with_metadata(true),
        }
    }
}
