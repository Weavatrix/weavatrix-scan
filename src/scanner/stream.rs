use super::{Scanner, discover_repository_with_options};
use crate::content::inspect_files;
use crate::error::Result;
use crate::scan_finalize::{RevisionBuilder, sort_report_evidence};
use crate::scan_stream::{ScanSink, ScanSinkControl, ScanStreamReport};

impl Scanner {
    /// Inspects and emits the deterministic manifest under synchronous
    /// backpressure without retaining selected file records.
    ///
    /// The sink is invoked in normalized relative-path order. Returning
    /// [`ScanSinkControl::Stop`] stops inspection and marks the stream summary
    /// incomplete.
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`Self::scan`].
    pub fn scan_into<S>(self, mut sink: S) -> Result<ScanStreamReport>
    where
        S: ScanSink,
    {
        let (mut report, runtime) = discover_repository_with_options(&self.root, &self.options)?;
        sort_report_evidence(&mut report);
        let files = std::mem::take(&mut report.files);
        let mut revision = RevisionBuilder::new(&report);
        let mut selected = 0_u64;
        let mut emitted = 0_u64;
        let mut stopped = false;
        for file in files {
            let inspected = inspect_files(vec![file], &self.options, runtime.started, None)?;
            report.skipped.extend(inspected.skipped);
            if !inspected.warnings.is_empty() {
                report.complete = false;
                report.warnings.extend(inspected.warnings);
            }
            report.cache.reused_hashes = report
                .cache
                .reused_hashes
                .saturating_add(inspected.cache.reused_hashes);
            report.cache.content_reads = report
                .cache
                .content_reads
                .saturating_add(inspected.cache.content_reads);
            report.cache.fingerprint_reads = report
                .cache
                .fingerprint_reads
                .saturating_add(inspected.cache.fingerprint_reads);
            if let Some(reason) = inspected.termination {
                report.terminate(reason);
            }
            for file in inspected.files {
                revision.push(&file);
                selected = selected.saturating_add(1);
                emitted = emitted.saturating_add(1);
                if sink.on_file(&file) == ScanSinkControl::Stop {
                    stopped = true;
                    report.complete = false;
                    break;
                }
            }
            if stopped || report.termination.is_some() {
                break;
            }
        }
        sort_report_evidence(&mut report);
        report.revision = revision.finish(&report);
        report.finish_recording();
        Ok(ScanStreamReport {
            root: report.root,
            selected,
            emitted,
            stopped,
            skipped: report.skipped,
            warnings: report.warnings,
            ignore_sources: report.ignore_sources,
            revision: report.revision,
            complete: report.complete,
            termination: report.termination,
            portable: report.portable,
            cache: report.cache,
        })
    }
}
