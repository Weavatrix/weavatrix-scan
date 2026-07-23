use std::path::PathBuf;

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScannedFile {
    pub absolute: PathBuf,
    pub relative: String,
    pub bytes: u64,
    pub content_hash: Option<String>,
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkipKind {
    Binary,
    Extension,
    Ignored,
    Oversized,
    PathEscape,
    StandardDirectory,
    Symlink,
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
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanReport {
    pub root: PathBuf,
    pub files: Vec<ScannedFile>,
    pub skipped: Vec<SkippedEntry>,
    pub warnings: Vec<ScanWarning>,
    pub revision: String,
}

impl ScanReport {
    pub(crate) fn new(root: PathBuf) -> Self {
        Self {
            root,
            files: Vec::new(),
            skipped: Vec::new(),
            warnings: Vec::new(),
            revision: String::new(),
        }
    }

    pub(crate) fn skip(&mut self, relative: String, kind: SkipKind, detail: Option<String>) {
        self.skipped.push(SkippedEntry {
            relative,
            kind,
            detail,
        });
    }

    pub(crate) fn warn(&mut self, relative: Option<String>, message: impl Into<String>) {
        self.warnings.push(ScanWarning {
            relative,
            message: message.into(),
        });
    }
}
