use super::{
    AtomicU8, Error, ErrorPolicy, EvidenceMode, InspectedFiles, Instant, Ordering, Result,
    ScanCacheEntry, ScanOptions, ScanTermination, ScanWarning, ScannedFile, SkipKind, SkippedEntry,
    io, reusable,
};

pub(super) fn reusable_candidate(
    current: &ScannedFile,
    options: &ScanOptions,
    previous: Option<&ScanCacheEntry>,
) -> bool {
    let Some(previous) = previous else {
        return false;
    };
    if !previous.content_hash.starts_with("sha256:") {
        return false;
    }
    if current.bytes != previous.bytes
        || !reusable(&previous.version, &current.version)
        || (options.detect_binary_files && !previous.binary_checked)
    {
        return false;
    }
    true
}

pub(super) fn apply_cached(current: &mut ScannedFile, previous: &ScanCacheEntry) {
    current.content_hash = Some(previous.content_hash.clone());
    current.content_fingerprint = Some(previous.content_fingerprint.clone());
    current.binary_checked = previous.binary_checked;
}

#[derive(Default)]
pub(super) struct InspectionStop(AtomicU8);

impl InspectionStop {
    pub(super) fn reason(
        &self,
        options: &ScanOptions,
        started: Instant,
    ) -> Option<ScanTermination> {
        if let Some(reason) = termination_from_code(self.0.load(Ordering::Acquire)) {
            return Some(reason);
        }
        let reason = if options
            .cancellation
            .as_ref()
            .is_some_and(crate::control::CancellationToken::is_cancelled)
        {
            Some(ScanTermination::Cancelled)
        } else if options
            .limits
            .timeout
            .is_some_and(|timeout| started.elapsed() >= timeout)
        {
            Some(ScanTermination::Timeout)
        } else {
            None
        };
        if let Some(reason) = reason {
            let code = termination_code(reason);
            let _ = self
                .0
                .compare_exchange(0, code, Ordering::AcqRel, Ordering::Acquire);
        }
        termination_from_code(self.0.load(Ordering::Acquire))
    }
}

pub(super) const fn termination_code(reason: ScanTermination) -> u8 {
    match reason {
        ScanTermination::Timeout => 1,
        ScanTermination::Cancelled => 2,
        ScanTermination::MaxEntries | ScanTermination::MaxTotalBytes => 0,
    }
}

const fn termination_from_code(code: u8) -> Option<ScanTermination> {
    match code {
        1 => Some(ScanTermination::Timeout),
        2 => Some(ScanTermination::Cancelled),
        _ => None,
    }
}

pub(super) fn record_limit_skip(
    inspected: &mut InspectedFiles,
    relative: String,
    reason: ScanTermination,
    options: &ScanOptions,
) {
    if options.evidence == EvidenceMode::Complete {
        inspected.skipped.push(SkippedEntry {
            relative,
            kind: SkipKind::ScanLimit,
            detail: Some(format!("{reason:?}")),
        });
    }
}

pub(super) fn binary_skip(relative: String) -> SkippedEntry {
    SkippedEntry {
        relative,
        kind: SkipKind::Binary,
        detail: None,
    }
}

pub(super) fn record_binary_skip(
    inspected: &mut InspectedFiles,
    relative: String,
    options: &ScanOptions,
) {
    if options.evidence == EvidenceMode::Complete {
        inspected.skipped.push(binary_skip(relative));
    }
}

pub(super) fn record_concurrent_modification(
    inspected: &mut InspectedFiles,
    relative: String,
    options: &ScanOptions,
) -> Result<()> {
    if options.walk.error_policy == ErrorPolicy::Abort {
        return Err(Error::concurrent_modification(relative));
    }
    let message = "file changed while the scan was reading it".to_owned();
    if options.evidence == EvidenceMode::Complete {
        inspected.skipped.push(SkippedEntry {
            relative: relative.clone(),
            kind: SkipKind::ConcurrentModification,
            detail: Some(message.clone()),
        });
    }
    inspected.warnings.push(ScanWarning {
        relative: Some(relative),
        message,
    });
    Ok(())
}

pub(super) fn record_io_error(
    inspected: &mut InspectedFiles,
    file: &ScannedFile,
    operation: &str,
    source: io::Error,
    options: &ScanOptions,
) -> Result<()> {
    if options.walk.error_policy == ErrorPolicy::Abort {
        return Err(Error::io(&file.absolute, source));
    }
    let message = format!("{operation}: {source}");
    if options.evidence == EvidenceMode::Complete {
        inspected.skipped.push(SkippedEntry {
            relative: file.relative.clone(),
            kind: SkipKind::IoError,
            detail: Some(message.clone()),
        });
    }
    inspected.warnings.push(ScanWarning {
        relative: Some(file.relative.clone()),
        message,
    });
    Ok(())
}
