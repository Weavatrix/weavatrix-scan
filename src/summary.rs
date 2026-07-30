use crate::report::{
    CompactScanReport, ScanCacheStats, ScanReport, ScanTermination, SkipKind, SkippedEntry,
};
use std::collections::BTreeMap;
use std::fmt;

/// Path-free aggregate scan output for logs, telemetry, and higher-level tools.
///
/// `recorded_skips` and `skipped_by_kind` describe retained evidence. They are
/// zero when the scan used [`crate::EvidenceMode::SelectedFiles`], even when
/// entries were excluded during selection.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanSummary {
    pub selected_files: usize,
    pub selected_bytes: u64,
    pub hashed_files: usize,
    pub binary_checked_files: usize,
    pub recorded_skips: usize,
    pub skipped_by_kind: BTreeMap<SkipKind, usize>,
    pub warnings: usize,
    pub ignore_sources: usize,
    pub complete: bool,
    pub termination: Option<ScanTermination>,
    pub portable: bool,
    pub cache: ScanCacheStats,
}

impl ScanSummary {
    #[allow(clippy::too_many_arguments)]
    fn from_parts(
        selected_files: usize,
        selected_bytes: impl Iterator<Item = u64>,
        hashed_files: usize,
        binary_checked_files: usize,
        skipped: &[SkippedEntry],
        warnings: usize,
        ignore_sources: usize,
        complete: bool,
        termination: Option<ScanTermination>,
        portable: bool,
        cache: ScanCacheStats,
    ) -> Self {
        let mut skipped_by_kind = BTreeMap::new();
        for entry in skipped {
            *skipped_by_kind.entry(entry.kind).or_default() += 1;
        }
        Self {
            selected_files,
            selected_bytes: selected_bytes.fold(0_u64, u64::saturating_add),
            hashed_files,
            binary_checked_files,
            recorded_skips: skipped.len(),
            skipped_by_kind,
            warnings,
            ignore_sources,
            complete,
            termination,
            portable,
            cache,
        }
    }
}

impl ScanReport {
    /// Aggregates this report without exposing repository paths.
    #[must_use]
    pub fn summary(&self) -> ScanSummary {
        ScanSummary::from_parts(
            self.files.len(),
            self.files.iter().map(|file| file.bytes),
            self.files
                .iter()
                .filter(|file| file.content_hash.is_some())
                .count(),
            self.files.iter().filter(|file| file.binary_checked).count(),
            &self.skipped,
            self.warnings.len(),
            self.ignore_sources.len(),
            self.complete,
            self.termination,
            self.portable,
            self.cache,
        )
    }
}

impl CompactScanReport {
    /// Aggregates this compact report without exposing repository paths.
    #[must_use]
    pub fn summary(&self) -> ScanSummary {
        ScanSummary::from_parts(
            self.files.len(),
            self.files.iter().map(|file| file.bytes),
            self.files
                .iter()
                .filter(|file| file.content_hash().is_some())
                .count(),
            self.files
                .iter()
                .filter(|file| {
                    file.content
                        .as_deref()
                        .is_some_and(|content| content.binary_checked)
                })
                .count(),
            &self.skipped,
            self.warnings.len(),
            self.ignore_sources.len(),
            self.complete,
            self.termination,
            self.portable,
            self.cache,
        )
    }
}

impl fmt::Display for ScanSummary {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "files={} bytes={} hashed={} binary_checked={} skipped={} warnings={} \
             ignore_sources={} complete={} portable={}",
            self.selected_files,
            self.selected_bytes,
            self.hashed_files,
            self.binary_checked_files,
            self.recorded_skips,
            self.warnings,
            self.ignore_sources,
            self.complete,
            self.portable,
        )?;
        if let Some(termination) = self.termination {
            write!(formatter, " termination={termination:?}")?;
        }
        if self.cache != ScanCacheStats::default() {
            write!(
                formatter,
                " cache_reused_hashes={} cache_content_reads={} cache_fingerprint_reads={}",
                self.cache.reused_hashes, self.cache.content_reads, self.cache.fingerprint_reads,
            )?;
        }
        Ok(())
    }
}
