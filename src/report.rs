use std::path::PathBuf;

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScannedFile {
    #[cfg_attr(feature = "serde", serde(with = "crate::path_serde"))]
    pub absolute: PathBuf,
    pub relative: String,
    pub bytes: u64,
    pub content_hash: Option<String>,
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
    #[cfg_attr(feature = "serde", serde(skip, default = "default_record_skipped"))]
    record_skipped: bool,
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
