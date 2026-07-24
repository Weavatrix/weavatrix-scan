use crate::hash::FingerprintHasher;
use crate::report::{ScanReport, ScannedFile};

pub(crate) fn finalize_report(report: &mut ScanReport) {
    sort_report_evidence(report);
    let mut revision = RevisionBuilder::new(report);
    for file in &report.files {
        revision.push(file);
    }
    report.revision = revision.finish(report);
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
    pub(crate) fn new(report: &ScanReport) -> Self {
        let mut revision = FingerprintHasher::new();
        for source in &report.ignore_sources {
            revision.write(format!("{:?}", source.kind).as_bytes());
            revision.write(&[0]);
            revision.write(source.location.as_bytes());
            revision.write(&[0]);
            revision.write(source.content_hash.as_bytes());
            revision.write(&[0xfe]);
        }
        Self { revision }
    }

    pub(crate) fn push(&mut self, file: &ScannedFile) {
        self.revision.write(file.relative.as_bytes());
        self.revision.write(&[0]);
        self.revision
            .write(file.content_hash.as_deref().unwrap_or("").as_bytes());
        self.revision.write(&[0xff]);
    }

    pub(crate) fn finish(mut self, report: &ScanReport) -> String {
        self.revision.write(&[u8::from(report.portable)]);
        if let Some(termination) = report.termination {
            self.revision.write(format!("{termination:?}").as_bytes());
        }
        self.revision.finish()
    }
}
