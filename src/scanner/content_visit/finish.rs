use super::{
    CompactScanReport, CompactScannedFile, ContentVisitMode, ContentVisitReport, ScanReport,
    VisitedFiles, compact_revision, sort_evidence,
};
use crate::report::ScanTermination;

pub(super) struct FinishedContentVisit {
    pub(super) report: ContentVisitReport,
    pub(super) files: Vec<CompactScannedFile>,
}

impl FinishedContentVisit {
    pub(super) fn into_manifest(self) -> CompactScanReport {
        CompactScanReport {
            root: self.report.root,
            files: self.files,
            skipped: self.report.skipped,
            warnings: self.report.warnings,
            ignore_sources: self.report.ignore_sources,
            revision: self.report.revision,
            complete: self.report.complete,
            termination: self.report.termination,
            portable: self.report.portable,
            cache: self.report.cache,
        }
    }
}

pub(super) fn finish_content_report(
    mut evidence: ScanReport,
    discovered: u64,
    worker_reports: Vec<VisitedFiles>,
    mode: ContentVisitMode,
) -> FinishedContentVisit {
    let mut selected = Vec::new();
    let mut totals = VisitedFiles::empty(0);
    for mut worker in worker_reports {
        selected.append(&mut worker.files);
        totals.merge(worker);
    }
    evidence.skipped.extend(totals.evidence.skipped);
    evidence.warnings.extend(totals.evidence.warnings);
    evidence.termination = evidence.termination.or(totals.evidence.termination);
    evidence.cache = totals.evidence.cache;
    if totals.visitor_quit {
        evidence.complete = false;
        evidence.termination = Some(ScanTermination::Cancelled);
    }
    if !evidence.warnings.is_empty() || evidence.termination.is_some() {
        evidence.complete = false;
    }
    sort_evidence(&mut evidence);
    let (revision, files) = if mode == ContentVisitMode::Revision {
        selected.sort_unstable_by(|left, right| left.1.relative.cmp(&right.1.relative));
        let files = selected
            .into_iter()
            .map(|(_, file)| file)
            .collect::<Vec<_>>();
        (compact_revision(&evidence, &files), files)
    } else {
        (String::new(), Vec::new())
    };
    let stopped = evidence.termination.is_some();
    evidence.finish_recording();
    FinishedContentVisit {
        report: ContentVisitReport {
            mode,
            root: evidence.root,
            discovered,
            completed: totals.completed,
            opened: totals.opened,
            chunks: totals.chunks,
            bytes_read: totals.bytes_read,
            bytes_emitted: totals.bytes_emitted,
            consumer_skipped: totals.consumer_skipped,
            stopped,
            skipped: evidence.skipped,
            warnings: evidence.warnings,
            ignore_sources: evidence.ignore_sources,
            revision,
            complete: evidence.complete,
            termination: evidence.termination,
            portable: evidence.portable,
            cache: evidence.cache,
        },
        files,
    }
}
