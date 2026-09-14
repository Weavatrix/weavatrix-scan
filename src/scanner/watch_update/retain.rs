use crate::path::{collapse_path_prefixes, path_covered_by_prefixes};
use crate::report::{ScanCacheStats, ScanReport};
use crate::watch::WatchPlan;

pub(super) fn prepare_incremental_report(previous: &ScanReport, plan: &WatchPlan) -> ScanReport {
    let prefixes = collapse_path_prefixes(plan.invalidated());
    let mut report = previous.clone();
    report
        .files
        .retain(|file| !path_covered_by_prefixes(&file.relative, &prefixes));
    report
        .skipped
        .retain(|entry| !path_covered_by_prefixes(&entry.relative, &prefixes));
    report.warnings.retain(|warning| {
        warning
            .relative
            .as_deref()
            .is_none_or(|relative| !path_covered_by_prefixes(relative, &prefixes))
    });
    report.revision.clear();
    report.complete = true;
    report.termination = None;
    report.cache = ScanCacheStats::default();
    report
}
