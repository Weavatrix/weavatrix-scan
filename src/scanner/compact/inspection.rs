use super::{
    CompactContentEvidence, CompactScannedFile, EvidenceMode, FingerprintHasher, Path, Result,
    ScanOptions, ScanReport, ScannedFile, SkipKind, SkippedEntry, inspect_files,
};
use crate::report::{FileVersion, ScanTermination};

pub(super) fn compact_file(
    relative: String,
    bytes: u64,
    version: FileVersion,
    options: &ScanOptions,
) -> CompactScannedFile {
    CompactScannedFile {
        relative: relative.into_boxed_str(),
        bytes,
        content: (options.hash_file_contents || options.detect_binary_files).then(|| {
            Box::new(CompactContentEvidence {
                content_hash: None,
                content_fingerprint: None,
                version,
                binary_checked: false,
            })
        }),
    }
}

pub(super) fn inspect_compact(
    root: &Path,
    files: Vec<CompactScannedFile>,
    options: &ScanOptions,
    started: std::time::Instant,
    evidence: &mut ScanReport,
) -> Result<Vec<CompactScannedFile>> {
    const CHUNK_SIZE: usize = 4_096;
    let mut iterator = files.into_iter();
    let mut inspected_files = Vec::new();
    loop {
        let chunk = iterator
            .by_ref()
            .take(CHUNK_SIZE)
            .map(|file| {
                let content = file
                    .content
                    .expect("content scan compact entry retains discovery evidence");
                ScannedFile {
                    absolute: root.join(file.relative.as_ref()),
                    relative: file.relative.into(),
                    bytes: file.bytes,
                    content_hash: content.content_hash.map(Into::into),
                    content_fingerprint: content.content_fingerprint.map(Into::into),
                    version: content.version,
                    binary_checked: content.binary_checked,
                }
            })
            .collect::<Vec<_>>();
        if chunk.is_empty() {
            break;
        }
        let inspected = inspect_files(chunk, options, started, None)?;
        inspected_files.extend(inspected.files.into_iter().map(|file| CompactScannedFile {
            relative: file.relative.into_boxed_str(),
            bytes: file.bytes,
            content: Some(Box::new(CompactContentEvidence {
                content_hash: file.content_hash.map(String::into_boxed_str),
                content_fingerprint: file.content_fingerprint.map(String::into_boxed_str),
                version: file.version,
                binary_checked: file.binary_checked,
            })),
        }));
        evidence.skipped.extend(inspected.skipped);
        evidence.warnings.extend(inspected.warnings);
        evidence.cache.reused_hashes = evidence
            .cache
            .reused_hashes
            .saturating_add(inspected.cache.reused_hashes);
        evidence.cache.content_reads = evidence
            .cache
            .content_reads
            .saturating_add(inspected.cache.content_reads);
        evidence.cache.fingerprint_reads = evidence
            .cache
            .fingerprint_reads
            .saturating_add(inspected.cache.fingerprint_reads);
        if let Some(reason) = inspected.termination {
            evidence.terminate(reason);
        }
        if !evidence.warnings.is_empty() {
            evidence.complete = false;
        }
        if let Some(reason) = evidence.termination {
            if options.evidence == EvidenceMode::Complete {
                evidence
                    .skipped
                    .extend(iterator.by_ref().map(|file| SkippedEntry {
                        relative: file.relative.into(),
                        kind: SkipKind::ScanLimit,
                        detail: Some(format!("content inspection stopped: {reason:?}")),
                    }));
            }
            break;
        }
    }
    Ok(inspected_files)
}

pub(crate) fn apply_total_bytes_limit(
    evidence: &mut ScanReport,
    files: &mut Vec<CompactScannedFile>,
    options: &ScanOptions,
) {
    let Some(limit) = options.limits.max_total_bytes else {
        return;
    };
    let mut total = 0_u64;
    let keep = files
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
        .unwrap_or(files.len());
    if keep == files.len() {
        return;
    }
    let truncated = files.split_off(keep);
    if options.evidence == EvidenceMode::Complete {
        evidence
            .skipped
            .extend(truncated.into_iter().map(|file| SkippedEntry {
                relative: file.relative.into(),
                kind: SkipKind::ScanLimit,
                detail: Some(format!("maximum selected bytes: {limit}")),
            }));
    }
    evidence.terminate(ScanTermination::MaxTotalBytes);
}

pub(crate) fn sort_evidence(report: &mut ScanReport) {
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
    report.ignore_sources.dedup();
}

pub(crate) fn compact_revision(evidence: &ScanReport, files: &[CompactScannedFile]) -> String {
    let mut revision = FingerprintHasher::new();
    for source in &evidence.ignore_sources {
        revision.write(format!("{:?}", source.kind).as_bytes());
        revision.write(&[0]);
        revision.write(source.location.as_bytes());
        revision.write(&[0]);
        revision.write(source.content_hash.as_bytes());
        revision.write(&[0xfe]);
    }
    for file in files {
        revision.write(file.relative.as_bytes());
        revision.write(&[0]);
        revision.write(file.content_hash().unwrap_or("").as_bytes());
        revision.write(&[0xff]);
    }
    revision.write(&[u8::from(evidence.portable)]);
    if let Some(termination) = evidence.termination {
        revision.write(format!("{termination:?}").as_bytes());
    }
    revision.finish()
}
