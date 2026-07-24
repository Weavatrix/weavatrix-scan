use crate::report::{FileVersion, ScanReport};
use std::path::{Path, PathBuf};

/// Current on-disk format understood by [`ScanCache`].
pub const SCAN_CACHE_FORMAT_VERSION: u32 = 2;

/// Compact reusable evidence for one content-hashed file.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanCacheEntry {
    pub relative: String,
    pub bytes: u64,
    pub content_hash: String,
    #[cfg_attr(feature = "serde", serde(default))]
    pub content_fingerprint: String,
    pub version: FileVersion,
    pub binary_checked: bool,
}

/// Versioned local cache for incremental scans.
///
/// Unlike [`ScanReport`], this contains no absolute per-file paths, skipped
/// entries, warnings, ignore diagnostics, or manifest metadata.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanCache {
    pub format_version: u32,
    #[cfg_attr(feature = "serde", serde(with = "crate::path_serde"))]
    pub root: PathBuf,
    pub entries: Vec<ScanCacheEntry>,
}

impl ScanCache {
    /// Builds a compact cache from reusable SHA-256 evidence.
    #[must_use]
    pub fn from_report(report: &ScanReport) -> Self {
        let entries = report
            .files
            .iter()
            .filter_map(|file| {
                let content_hash = file
                    .content_hash
                    .as_ref()
                    .filter(|hash| hash.starts_with("sha256:"))?
                    .clone();
                Some(ScanCacheEntry {
                    relative: file.relative.clone(),
                    bytes: file.bytes,
                    content_hash,
                    content_fingerprint: file.content_fingerprint.clone()?,
                    version: file.version,
                    binary_checked: file.binary_checked,
                })
            })
            .collect();
        Self {
            format_version: SCAN_CACHE_FORMAT_VERSION,
            root: report.root.clone(),
            entries,
        }
    }

    /// Returns whether this cache can be considered for `root`.
    #[must_use]
    pub fn is_compatible(&self, root: &Path) -> bool {
        self.format_version == SCAN_CACHE_FORMAT_VERSION && self.root == root
    }

    /// Removes reusable entries for the supplied normalized relative paths.
    ///
    /// Returns the number of entries removed.
    pub fn invalidate<'a, I>(&mut self, relative_paths: I) -> usize
    where
        I: IntoIterator<Item = &'a str>,
    {
        let paths = relative_paths
            .into_iter()
            .collect::<std::collections::HashSet<_>>();
        let before = self.entries.len();
        self.entries
            .retain(|entry| !paths.contains(entry.relative.as_str()));
        before - self.entries.len()
    }

    /// Applies a watcher plan, clearing everything when selection may change.
    ///
    /// Returns the number of entries removed.
    pub fn apply_watch_plan(&mut self, plan: &crate::WatchPlan) -> usize {
        if plan.full_rescan {
            let removed = self.entries.len();
            self.entries.clear();
            return removed;
        }
        self.invalidate(plan.invalidated())
    }
}

impl ScanReport {
    /// Extracts the compact, versioned evidence needed for a later scan.
    #[must_use]
    pub fn to_cache(&self) -> ScanCache {
        ScanCache::from_report(self)
    }
}
