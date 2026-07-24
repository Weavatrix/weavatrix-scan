use crate::file_version;
use crate::hash::FingerprintHasher;
use crate::path::normalized_relative_path;
use crate::report::{ScanReport, ScannedFile};
use std::fmt;
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::Path;

/// Evidence used to bind returned bytes to a scan snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapshotEvidence {
    FileVersion,
    Sha256,
}

/// File bytes verified against the selected scan entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotContent {
    pub bytes: Vec<u8>,
    pub evidence: SnapshotEvidence,
}

/// Reads only files recorded in one validated [`ScanReport`].
#[derive(Debug, Clone, Copy)]
pub struct SnapshotContentProvider<'a> {
    root: &'a Path,
    files: &'a [ScannedFile],
}

/// A path-safe failure from snapshot content access.
#[derive(Debug)]
pub enum SnapshotReadError {
    InvalidReport {
        relative: Option<String>,
        reason: &'static str,
    },
    UnknownFile(String),
    Io {
        relative: String,
        source: io::Error,
    },
    LimitExceeded {
        relative: String,
        bytes: u64,
        max_bytes: u64,
    },
    Stale(String),
}

impl ScanReport {
    /// Creates a provider after validating report ordering and path scope.
    ///
    /// # Errors
    ///
    /// Returns [`SnapshotReadError::InvalidReport`] for a malformed,
    /// path-escaping, duplicate, or unverifiable report entry.
    pub fn content_provider(&self) -> Result<SnapshotContentProvider<'_>, SnapshotReadError> {
        SnapshotContentProvider::new(self)
    }
}

impl<'a> SnapshotContentProvider<'a> {
    /// Validates a scan report before any content is opened.
    ///
    /// # Errors
    ///
    /// Returns [`SnapshotReadError::InvalidReport`] when report entries are
    /// not strictly sorted, escape the root, or lack snapshot evidence.
    pub fn new(report: &'a ScanReport) -> Result<Self, SnapshotReadError> {
        validate_report(report)?;
        Ok(Self {
            root: &report.root,
            files: &report.files,
        })
    }

    /// Reads a selected file and verifies it before and after the read.
    ///
    /// # Errors
    ///
    /// Returns `UnknownFile` for paths outside this snapshot, `Stale` when the
    /// file changed or became a link, and `Io` for independent read failures.
    pub fn read(&self, relative: &str) -> Result<SnapshotContent, SnapshotReadError> {
        self.read_bounded(relative, u64::MAX)
    }

    /// Reads a selected file only when its recorded size is within `max_bytes`.
    ///
    /// # Errors
    ///
    /// Returns the same failures as [`Self::read`], plus `LimitExceeded`.
    pub fn read_bounded(
        &self,
        relative: &str,
        max_bytes: u64,
    ) -> Result<SnapshotContent, SnapshotReadError> {
        let index = self
            .files
            .binary_search_by(|file| file.relative.as_str().cmp(relative))
            .map_err(|_| SnapshotReadError::UnknownFile(relative.to_owned()))?;
        let snapshot = &self.files[index];
        if snapshot.bytes > max_bytes {
            return Err(SnapshotReadError::LimitExceeded {
                relative: relative.to_owned(),
                bytes: snapshot.bytes,
                max_bytes,
            });
        }
        self.read_file(snapshot)
    }

    fn read_file(&self, snapshot: &ScannedFile) -> Result<SnapshotContent, SnapshotReadError> {
        let relative = snapshot.relative.as_str();
        let expected_bytes =
            usize::try_from(snapshot.bytes).map_err(|_| SnapshotReadError::LimitExceeded {
                relative: relative.to_owned(),
                bytes: snapshot.bytes,
                max_bytes: u64::try_from(usize::MAX).unwrap_or(u64::MAX),
            })?;
        let link_metadata =
            fs::symlink_metadata(&snapshot.absolute).map_err(|error| map_io(relative, error))?;
        if link_metadata.file_type().is_symlink() {
            return Err(SnapshotReadError::Stale(relative.to_owned()));
        }
        let canonical = snapshot
            .absolute
            .canonicalize()
            .map_err(|error| map_io(relative, error))?;
        if !canonical.starts_with(self.root) {
            return Err(SnapshotReadError::Stale(relative.to_owned()));
        }
        let mut file = File::open(&snapshot.absolute).map_err(|error| map_io(relative, error))?;
        let before_metadata = file.metadata().map_err(|error| map_io(relative, error))?;
        if !before_metadata.is_file() || before_metadata.len() != snapshot.bytes {
            return Err(SnapshotReadError::Stale(relative.to_owned()));
        }
        let before = file_version::from_file(&file, &before_metadata)
            .map_err(|error| map_io(relative, error))?;
        let version_matches = file_version::reusable(&snapshot.version, &before);
        if !version_matches && snapshot.content_hash.is_none() {
            return Err(SnapshotReadError::Stale(relative.to_owned()));
        }

        let read_limit = snapshot.bytes.saturating_add(1);
        let mut bytes = Vec::new();
        (&mut file)
            .take(read_limit)
            .read_to_end(&mut bytes)
            .map_err(|error| map_io(relative, error))?;
        if bytes.len() != expected_bytes {
            return Err(SnapshotReadError::Stale(relative.to_owned()));
        }

        let after_metadata = file.metadata().map_err(|error| map_io(relative, error))?;
        let after = file_version::from_file(&file, &after_metadata)
            .map_err(|error| map_io(relative, error))?;
        if !file_version::reusable(&before, &after) {
            return Err(SnapshotReadError::Stale(relative.to_owned()));
        }

        let evidence = if let Some(expected_hash) = &snapshot.content_hash {
            let mut hash = FingerprintHasher::new();
            hash.write(&bytes);
            if hash.finish() != *expected_hash {
                return Err(SnapshotReadError::Stale(relative.to_owned()));
            }
            SnapshotEvidence::Sha256
        } else {
            SnapshotEvidence::FileVersion
        };
        Ok(SnapshotContent { bytes, evidence })
    }
}

impl fmt::Display for SnapshotReadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidReport { relative, reason } => {
                write!(
                    formatter,
                    "invalid scan report entry {}: {reason}",
                    relative.as_deref().unwrap_or("<root>")
                )
            }
            Self::UnknownFile(relative) => {
                write!(
                    formatter,
                    "file is not present in the scan snapshot: {relative}"
                )
            }
            Self::Io { relative, source } => {
                write!(
                    formatter,
                    "could not read scan snapshot file {relative}: {source}"
                )
            }
            Self::LimitExceeded {
                relative,
                bytes,
                max_bytes,
            } => write!(
                formatter,
                "scan snapshot file exceeds content limit: {relative} ({bytes} > {max_bytes})"
            ),
            Self::Stale(relative) => write!(formatter, "scan snapshot is stale: {relative}"),
        }
    }
}

impl std::error::Error for SnapshotReadError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::InvalidReport { .. }
            | Self::UnknownFile(_)
            | Self::LimitExceeded { .. }
            | Self::Stale(_) => None,
        }
    }
}

fn validate_report(report: &ScanReport) -> Result<(), SnapshotReadError> {
    if !report.root.is_absolute() {
        return Err(invalid(None, "root is not absolute"));
    }
    let mut previous: Option<&str> = None;
    for file in &report.files {
        if previous.is_some_and(|value| value >= file.relative.as_str()) {
            return Err(invalid(
                Some(file.relative.clone()),
                "files are not strictly sorted",
            ));
        }
        previous = Some(&file.relative);
        let relative = file
            .absolute
            .strip_prefix(&report.root)
            .map_err(|_| invalid(Some(file.relative.clone()), "absolute path escapes root"))?;
        if normalized_relative_path(relative) != file.relative {
            return Err(invalid(
                Some(file.relative.clone()),
                "relative and absolute paths disagree",
            ));
        }
        if file.content_hash.is_none() && file.version.modified_ns.is_none() {
            return Err(invalid(
                Some(file.relative.clone()),
                "entry has no reusable snapshot evidence",
            ));
        }
    }
    Ok(())
}

fn invalid(relative: Option<String>, reason: &'static str) -> SnapshotReadError {
    SnapshotReadError::InvalidReport { relative, reason }
}

fn map_io(relative: &str, source: io::Error) -> SnapshotReadError {
    if source.kind() == io::ErrorKind::NotFound {
        SnapshotReadError::Stale(relative.to_owned())
    } else {
        SnapshotReadError::Io {
            relative: relative.to_owned(),
            source,
        }
    }
}
