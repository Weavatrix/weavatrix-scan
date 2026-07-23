use crate::path::RevisionHasher;
use crate::report::{ScanReport, ScannedFile};

pub(crate) fn finalize_report(report: &mut ScanReport) {
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
    report.revision = revision_for(&report.files);
}

fn revision_for(files: &[ScannedFile]) -> String {
    let mut revision = RevisionHasher::new();
    for file in files {
        revision.write(file.relative.as_bytes());
        revision.write(&[0]);
        revision.write(file.content_hash.as_deref().unwrap_or("").as_bytes());
        revision.write(&[0xff]);
    }
    revision.finish()
}
