use std::path::PathBuf;

mod compact;
mod scan;

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileIdentity {
    pub file_system: u64,
    pub file: u64,
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FileVersion {
    #[cfg_attr(feature = "serde", serde(default))]
    pub modified_ns: Option<u128>,
    #[cfg_attr(feature = "serde", serde(default))]
    pub changed_ns: Option<u128>,
    #[cfg_attr(feature = "serde", serde(default))]
    pub identity: Option<FileIdentity>,
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScannedFile {
    #[cfg_attr(feature = "serde", serde(with = "crate::path_serde"))]
    pub absolute: PathBuf,
    pub relative: String,
    pub bytes: u64,
    pub content_hash: Option<String>,
    /// Whole-content validation fingerprint used only for strict cache reuse.
    #[cfg_attr(feature = "serde", serde(default))]
    pub content_fingerprint: Option<String>,
    #[cfg_attr(feature = "serde", serde(default))]
    pub version: FileVersion,
    #[cfg_attr(feature = "serde", serde(default))]
    pub binary_checked: bool,
}

/// Selected-file evidence without a duplicated absolute path allocation.
///
/// Join `relative` to [`CompactScanReport::root`] only when an absolute path
/// is needed.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompactScannedFile {
    pub relative: Box<str>,
    pub bytes: u64,
    /// Allocated only when content inspection was requested.
    #[cfg_attr(feature = "serde", serde(default))]
    pub content: Option<Box<CompactContentEvidence>>,
}

/// Optional rich evidence kept out of metadata-only compact entries.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompactContentEvidence {
    pub content_hash: Option<Box<str>>,
    /// Whole-content validation fingerprint used only for strict cache reuse.
    #[cfg_attr(feature = "serde", serde(default))]
    pub content_fingerprint: Option<Box<str>>,
    #[cfg_attr(feature = "serde", serde(default))]
    pub version: FileVersion,
    #[cfg_attr(feature = "serde", serde(default))]
    pub binary_checked: bool,
}

impl CompactScannedFile {
    /// Returns the strong content hash when content hashing was requested.
    #[must_use]
    pub fn content_hash(&self) -> Option<&str> {
        self.content
            .as_deref()
            .and_then(|content| content.content_hash.as_deref())
    }
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ScanCacheStats {
    pub reused_hashes: u64,
    pub content_reads: u64,
    /// Whole-file fingerprint reads used to validate cached SHA-256 values.
    #[cfg_attr(feature = "serde", serde(default))]
    pub fingerprint_reads: u64,
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SkipKind {
    Binary,
    FileSystemBoundary,
    Extension,
    Ignored,
    IoError,
    MaxDepth,
    Oversized,
    PathEscape,
    StandardDirectory,
    Hidden,
    Override,
    Symlink,
    SymlinkLoop,
    ScanLimit,
    ConcurrentModification,
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkippedEntry {
    pub relative: String,
    pub kind: SkipKind,
    pub detail: Option<String>,
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanWarning {
    pub relative: Option<String>,
    pub message: String,
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum IgnoreSourceKind {
    GitGlobal,
    GitExclude,
    GitIgnore,
    DotIgnore,
    Custom,
    Explicit,
    Override,
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IgnoreSourceEvidence {
    pub kind: IgnoreSourceKind,
    pub location: String,
    pub content_hash: String,
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScanTermination {
    MaxEntries,
    MaxTotalBytes,
    Timeout,
    Cancelled,
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanReport {
    #[cfg_attr(feature = "serde", serde(with = "crate::path_serde"))]
    pub root: PathBuf,
    pub files: Vec<ScannedFile>,
    pub skipped: Vec<SkippedEntry>,
    pub warnings: Vec<ScanWarning>,
    /// Every ignore input that participated in path selection.
    #[cfg_attr(feature = "serde", serde(default))]
    pub ignore_sources: Vec<IgnoreSourceEvidence>,
    pub revision: String,
    /// False when local I/O or ignore-rule errors made evidence partial.
    #[cfg_attr(feature = "serde", serde(default = "default_complete"))]
    pub complete: bool,
    /// Why a bounded scan stopped before exhausting the tree.
    #[cfg_attr(feature = "serde", serde(default))]
    pub termination: Option<ScanTermination>,
    /// False when selection depended on host-level configuration.
    #[cfg_attr(feature = "serde", serde(default = "default_portable"))]
    pub portable: bool,
    /// Evidence of content work reused from an older persistent report.
    #[cfg_attr(feature = "serde", serde(default))]
    pub cache: ScanCacheStats,
    #[cfg_attr(feature = "serde", serde(skip, default = "default_record_skipped"))]
    record_skipped: bool,
}

/// Memory-efficient deterministic manifest retaining the root path once.
///
/// This report preserves scanner selection, hashes, revision, warnings and
/// typed skip evidence while avoiding one absolute `PathBuf` per selected
/// file.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompactScanReport {
    #[cfg_attr(feature = "serde", serde(with = "crate::path_serde"))]
    pub root: PathBuf,
    pub files: Vec<CompactScannedFile>,
    pub skipped: Vec<SkippedEntry>,
    pub warnings: Vec<ScanWarning>,
    #[cfg_attr(feature = "serde", serde(default))]
    pub ignore_sources: Vec<IgnoreSourceEvidence>,
    pub revision: String,
    #[cfg_attr(feature = "serde", serde(default = "default_complete"))]
    pub complete: bool,
    #[cfg_attr(feature = "serde", serde(default))]
    pub termination: Option<ScanTermination>,
    #[cfg_attr(feature = "serde", serde(default = "default_portable"))]
    pub portable: bool,
    #[cfg_attr(feature = "serde", serde(default))]
    pub cache: ScanCacheStats,
}

#[cfg(feature = "serde")]
const fn default_complete() -> bool {
    true
}

#[cfg(feature = "serde")]
const fn default_record_skipped() -> bool {
    true
}

#[cfg(feature = "serde")]
const fn default_portable() -> bool {
    true
}
