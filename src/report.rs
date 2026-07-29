use std::path::PathBuf;

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

impl CompactScanReport {
    /// Materializes an absolute path for one compact entry.
    #[must_use]
    pub fn absolute_path(&self, file: &CompactScannedFile) -> PathBuf {
        self.root.join(file.relative.as_ref())
    }

    /// Materializes the compatibility report without reading file contents
    /// again.
    #[must_use]
    pub fn into_scan_report(self) -> ScanReport {
        let root = self.root;
        let files = self
            .files
            .into_iter()
            .map(|file| {
                let content = file.content.map(|content| *content);
                ScannedFile {
                    absolute: root.join(file.relative.as_ref()),
                    relative: file.relative.into(),
                    bytes: file.bytes,
                    content_hash: content
                        .as_ref()
                        .and_then(|value| value.content_hash.as_deref())
                        .map(str::to_owned),
                    content_fingerprint: content
                        .as_ref()
                        .and_then(|value| value.content_fingerprint.as_deref())
                        .map(str::to_owned),
                    version: content
                        .as_ref()
                        .map_or_else(FileVersion::default, |value| value.version),
                    binary_checked: content.is_some_and(|value| value.binary_checked),
                }
            })
            .collect();
        ScanReport {
            root,
            files,
            skipped: self.skipped,
            warnings: self.warnings,
            ignore_sources: self.ignore_sources,
            revision: self.revision,
            complete: self.complete,
            termination: self.termination,
            portable: self.portable,
            cache: self.cache,
            record_skipped: true,
        }
    }
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

impl ScanReport {
    /// Computes the deterministic changed-file set from an older report.
    #[must_use]
    pub fn delta_from(&self, previous: &Self) -> crate::ScanDelta {
        crate::ScanDelta::between(previous, self)
    }

    pub(crate) fn new(root: PathBuf, record_skipped: bool) -> Self {
        Self {
            root,
            files: Vec::new(),
            skipped: Vec::new(),
            warnings: Vec::new(),
            ignore_sources: Vec::new(),
            revision: String::new(),
            complete: true,
            termination: None,
            portable: true,
            cache: ScanCacheStats::default(),
            record_skipped,
        }
    }

    pub(crate) fn skip(&mut self, relative: String, kind: SkipKind, detail: Option<String>) {
        if self.record_skipped {
            self.skipped.push(SkippedEntry {
                relative,
                kind,
                detail,
            });
        }
    }

    pub(crate) fn skip_borrowed(&mut self, relative: &str, kind: SkipKind, detail: Option<String>) {
        if self.record_skipped {
            self.skipped.push(SkippedEntry {
                relative: relative.to_owned(),
                kind,
                detail,
            });
        }
    }

    pub(crate) fn warn(&mut self, relative: Option<String>, message: impl Into<String>) {
        self.complete = false;
        self.warnings.push(ScanWarning {
            relative,
            message: message.into(),
        });
    }

    pub(crate) fn terminate(&mut self, reason: ScanTermination) {
        self.complete = false;
        self.termination.get_or_insert(reason);
    }

    pub(crate) fn finish_recording(&mut self) {
        self.record_skipped = true;
    }
}
