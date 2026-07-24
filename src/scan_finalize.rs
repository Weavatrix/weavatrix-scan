use crate::hash::FingerprintHasher;
use crate::report::ScanReport;

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
    report.ignore_sources.sort_unstable_by(|left, right| {
        left.kind
            .cmp(&right.kind)
            .then_with(|| left.location.cmp(&right.location))
            .then_with(|| left.content_hash.cmp(&right.content_hash))
    });
    report.ignore_sources.dedup_by(|left, right| left == right);
    report.revision = revision_for(report);
    report.finish_recording();
}

fn revision_for(report: &ScanReport) -> String {
    let mut revision = FingerprintHasher::new();
    for source in &report.ignore_sources {
        revision.write(format!("{:?}", source.kind).as_bytes());
        revision.write(&[0]);
        revision.write(source.location.as_bytes());
        revision.write(&[0]);
        revision.write(source.content_hash.as_bytes());
        revision.write(&[0xfe]);
    }
    for file in &report.files {
        revision.write(file.relative.as_bytes());
        revision.write(&[0]);
        revision.write(file.content_hash.as_deref().unwrap_or("").as_bytes());
        revision.write(&[0xff]);
    }
    revision.write(&[u8::from(report.portable)]);
    if let Some(termination) = report.termination {
        revision.write(format!("{termination:?}").as_bytes());
    }
    revision.finish()
}
