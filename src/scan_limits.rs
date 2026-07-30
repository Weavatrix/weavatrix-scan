use crate::config::{EvidenceMode, ScanOptions};
use crate::control::CancellationToken;
use crate::report::{ScanReport, ScanTermination, SkipKind, SkippedEntry};
use std::time::Instant;

pub(crate) struct ScanRuntime {
    pub(crate) started: Instant,
    entries: u64,
}

impl ScanRuntime {
    pub(crate) fn new() -> Self {
        Self {
            started: Instant::now(),
            entries: 0,
        }
    }

    pub(crate) fn before_next(&self, options: &ScanOptions) -> Option<ScanTermination> {
        if let Some(reason) = self.external_termination(options) {
            return Some(reason);
        }
        if options
            .limits
            .max_entries
            .is_some_and(|limit| self.entries >= limit)
        {
            return Some(ScanTermination::MaxEntries);
        }
        None
    }

    pub(crate) fn record_entry(&mut self) {
        self.entries = self.entries.saturating_add(1);
    }

    pub(crate) fn external_termination(&self, options: &ScanOptions) -> Option<ScanTermination> {
        if options
            .cancellation
            .as_ref()
            .is_some_and(CancellationToken::is_cancelled)
        {
            return Some(ScanTermination::Cancelled);
        }
        if options
            .limits
            .timeout
            .is_some_and(|timeout| self.started.elapsed() >= timeout)
        {
            return Some(ScanTermination::Timeout);
        }
        None
    }
}

pub(crate) fn apply_total_bytes_limit(report: &mut ScanReport, options: &ScanOptions) {
    let Some(limit) = options.limits.max_total_bytes else {
        return;
    };
    report
        .files
        .sort_unstable_by(|left, right| left.relative.cmp(&right.relative));
    let mut total = 0_u64;
    let keep = report
        .files
        .iter()
        .position(|file| {
            let next = total.saturating_add(file.bytes);
            if next > limit {
                true
            } else {
                total = next;
                false
            }
        })
        .unwrap_or(report.files.len());
    if keep == report.files.len() {
        return;
    }
    let truncated = report.files.split_off(keep);
    if options.evidence == EvidenceMode::Complete {
        report
            .skipped
            .extend(truncated.into_iter().map(|file| SkippedEntry {
                relative: file.relative,
                kind: SkipKind::ScanLimit,
                detail: Some(format!("maximum selected bytes: {limit}")),
            }));
    }
    report.terminate(ScanTermination::MaxTotalBytes);
}
