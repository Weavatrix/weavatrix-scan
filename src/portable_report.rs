use crate::hash::FingerprintHasher;
use crate::report::{
    IgnoreSourceEvidence, IgnoreSourceKind, ScanReport, ScanTermination, ScanWarning, ScannedFile,
    SkipKind, SkippedEntry,
};
use std::path::{Component, Path};

/// A selected file without host-local absolute paths or file identities.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortableScannedFile {
    pub relative: String,
    pub bytes: u64,
    pub content_hash: Option<String>,
    pub binary_checked: bool,
}

/// Typed skip evidence with free-form details replaced by a stable hash.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortableSkippedEntry {
    pub relative: String,
    pub kind: SkipKind,
    pub detail_hash: Option<String>,
}

/// Warning evidence that cannot expose paths embedded in an OS error message.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortableScanWarning {
    pub relative: Option<String>,
    pub message_hash: String,
}

/// Ignore-source evidence with external host paths removed.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortableIgnoreSourceEvidence {
    pub kind: IgnoreSourceKind,
    pub repository_relative: Option<String>,
    pub content_hash: String,
}

/// A deterministic report suitable for crossing a repository or process boundary.
///
/// Absolute roots, absolute file paths, file identities, timestamps, cache
/// statistics, and free-form diagnostic text are intentionally omitted.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortableScanReport {
    pub files: Vec<PortableScannedFile>,
    pub skipped: Vec<PortableSkippedEntry>,
    pub warnings: Vec<PortableScanWarning>,
    pub ignore_sources: Vec<PortableIgnoreSourceEvidence>,
    pub revision: String,
    #[cfg_attr(feature = "serde", serde(default))]
    pub descriptor: crate::ScanDescriptor,
    pub complete: bool,
    pub termination: Option<ScanTermination>,
    /// Whether selection itself was independent of host-level Git configuration.
    pub selection_portable: bool,
}

impl ScanReport {
    /// Returns path-safe evidence for IPC, logs, caches, and external tools.
    #[must_use]
    pub fn to_portable(&self) -> PortableScanReport {
        PortableScanReport::from(self)
    }

    /// Converts one page of selected files without rebuilding the whole report.
    #[must_use]
    pub fn portable_files_page(&self, offset: usize, limit: usize) -> Vec<PortableScannedFile> {
        let end = offset.saturating_add(limit);
        self.files
            .get(offset..self.files.len().min(end))
            .unwrap_or(&[])
            .iter()
            .map(PortableScannedFile::from)
            .collect()
    }
}

impl From<&ScanReport> for PortableScanReport {
    fn from(report: &ScanReport) -> Self {
        let mut ignore_sources = report
            .ignore_sources
            .iter()
            .map(PortableIgnoreSourceEvidence::from)
            .collect::<Vec<_>>();
        ignore_sources.sort_unstable_by(|left, right| {
            left.kind
                .cmp(&right.kind)
                .then_with(|| left.repository_relative.cmp(&right.repository_relative))
                .then_with(|| left.content_hash.cmp(&right.content_hash))
        });
        ignore_sources.dedup();
        let mut portable = Self {
            files: report.files.iter().map(PortableScannedFile::from).collect(),
            skipped: report
                .skipped
                .iter()
                .map(PortableSkippedEntry::from)
                .collect(),
            warnings: report
                .warnings
                .iter()
                .map(PortableScanWarning::from)
                .collect(),
            ignore_sources,
            revision: String::new(),
            descriptor: report.descriptor.clone(),
            complete: report.complete,
            termination: report.termination,
            selection_portable: report.portable,
        };
        portable.revision = portable_revision(&portable);
        portable
    }
}

impl From<&ScannedFile> for PortableScannedFile {
    fn from(file: &ScannedFile) -> Self {
        Self {
            relative: file.relative.clone(),
            bytes: file.bytes,
            content_hash: file.content_hash.clone(),
            binary_checked: file.binary_checked,
        }
    }
}

impl From<&SkippedEntry> for PortableSkippedEntry {
    fn from(skipped: &SkippedEntry) -> Self {
        Self {
            relative: skipped.relative.clone(),
            kind: skipped.kind,
            detail_hash: skipped.detail.as_deref().map(hash_text),
        }
    }
}

impl From<&ScanWarning> for PortableScanWarning {
    fn from(warning: &ScanWarning) -> Self {
        Self {
            relative: warning.relative.clone(),
            message_hash: hash_text(&warning.message),
        }
    }
}

impl From<&IgnoreSourceEvidence> for PortableIgnoreSourceEvidence {
    fn from(source: &IgnoreSourceEvidence) -> Self {
        Self {
            kind: source.kind,
            repository_relative: repository_relative_location(&source.location),
            content_hash: source.content_hash.clone(),
        }
    }
}

fn repository_relative_location(location: &str) -> Option<String> {
    if location.starts_with('<') && location.ends_with('>') {
        return Some(location.to_owned());
    }
    if looks_absolute(location) {
        return None;
    }
    let path = Path::new(location);
    if path
        .components()
        .any(|component| !matches!(component, Component::Normal(_)))
    {
        return None;
    }
    Some(location.replace('\\', "/"))
}

fn looks_absolute(location: &str) -> bool {
    let bytes = location.as_bytes();
    location.starts_with(['/', '\\'])
        || bytes.get(1) == Some(&b':') && bytes.first().is_some_and(u8::is_ascii_alphabetic)
}

fn hash_text(text: &str) -> String {
    let mut hash = FingerprintHasher::new();
    hash.write(text.as_bytes());
    hash.finish()
}

fn portable_revision(report: &PortableScanReport) -> String {
    let mut hasher = FingerprintHasher::new();
    hasher.write(b"scan-revision\x01");
    for source in &report.ignore_sources {
        crate::scan_identity::write_ignore_kind(&mut hasher, source.kind);
        hasher.write(&[0]);
        hasher.write(
            source
                .repository_relative
                .as_deref()
                .unwrap_or("<external>")
                .as_bytes(),
        );
        hasher.write(&[0]);
        hasher.write(source.content_hash.as_bytes());
        hasher.write(&[0xfe]);
    }
    for file in &report.files {
        crate::scan_identity::write_selected_file(
            &mut hasher,
            &file.relative,
            file.bytes,
            file.content_hash.as_deref(),
        );
    }
    hasher.write(&[u8::from(report.selection_portable)]);
    if let Some(termination) = report.termination {
        crate::scan_identity::write_termination(&mut hasher, termination);
    }
    hasher.finish()
}
