use crate::config::ScanOptions;
use crate::hash::FingerprintHasher;
use crate::report::{IgnoreSourceEvidence, ScanReport, ScannedFile};
use crate::scan_identity::{
    ScanDescriptor, write_ignore_kind, write_selected_file, write_termination,
};

pub(crate) fn finalize_report(report: &mut ScanReport, options: &ScanOptions) {
    sort_report_evidence(report);
    report.descriptor = ScanDescriptor::from_options(options);
    let mut revision = RevisionBuilder::new(&report.ignore_sources);
    for file in &report.files {
        revision.push(file);
    }
    report.revision = revision.finish(report.portable, report.termination);
    report.finish_recording();
}

pub(crate) fn sort_report_evidence(report: &mut ScanReport) {
    report
        .files
        .sort_unstable_by(|left, right| left.relative.cmp(&right.relative));
    report.skipped.sort_unstable_by(|left, right| {
        left.relative
            .cmp(&right.relative)
            .then_with(|| left.kind.cmp(&right.kind))
            .then_with(|| left.detail.cmp(&right.detail))
    });
    report.warnings.sort_unstable_by(|left, right| {
        left.relative
            .cmp(&right.relative)
            .then_with(|| left.message.cmp(&right.message))
    });
    report.ignore_sources.sort_unstable_by(|left, right| {
        left.kind
            .cmp(&right.kind)
            .then_with(|| left.location.cmp(&right.location))
            .then_with(|| left.content_hash.cmp(&right.content_hash))
    });
    report.ignore_sources.dedup_by(|left, right| left == right);
}

pub(crate) struct RevisionBuilder {
    revision: FingerprintHasher,
}

impl RevisionBuilder {
    pub(crate) fn new(sources: &[IgnoreSourceEvidence]) -> Self {
        let mut revision = FingerprintHasher::new();
        revision.write(b"scan-revision\x01");
        for source in sources {
            write_ignore_kind(&mut revision, source.kind);
            revision.write(&[0]);
            revision.write(source.location.as_bytes());
            revision.write(&[0]);
            revision.write(source.content_hash.as_bytes());
            revision.write(&[0xfe]);
        }
        Self { revision }
    }

    pub(crate) fn push(&mut self, file: &ScannedFile) {
        self.push_entry(&file.relative, file.bytes, file.content_hash.as_deref());
    }

    pub(crate) fn push_entry(&mut self, relative: &str, bytes: u64, content_hash: Option<&str>) {
        write_selected_file(&mut self.revision, relative, bytes, content_hash);
    }

    pub(crate) fn finish(
        mut self,
        portable: bool,
        termination: Option<crate::report::ScanTermination>,
    ) -> String {
        self.revision.write(&[u8::from(portable)]);
        if let Some(termination) = termination {
            write_termination(&mut self.revision, termination);
        }
        self.revision.finish()
    }
}
