#![allow(clippy::wildcard_imports)]

use super::*;

pub(super) fn content_file<'a>(
    root: &'a Path,
    root_index: usize,
    scanned: &'a ScannedFile,
    sequence: u64,
) -> ContentFile<'a> {
    ContentFile {
        root_index,
        sequence,
        root,
        absolute: &scanned.absolute,
        relative: &scanned.relative,
        bytes: scanned.bytes,
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn emit_end<V>(
    visitor: &mut V,
    worker_index: usize,
    root: &Path,
    root_index: usize,
    scanned: &ScannedFile,
    sequence: u64,
    status: ContentFileStatus,
    bytes_read: u64,
    content_hash: Option<&str>,
    consumer_skipped: bool,
    options: &ScanOptions,
) -> bool
where
    V: for<'event> FnMut(ContentVisitEvent<'event>) -> ContentVisitControl,
{
    if visitor(ContentVisitEvent::FileEnd {
        worker_index,
        file: content_file(root, root_index, scanned, sequence),
        status,
        bytes_read,
        content_hash,
        consumer_skipped,
    }) == ContentVisitControl::Quit
    {
        cancel(options);
        true
    } else {
        false
    }
}

pub(super) fn cancel(options: &ScanOptions) {
    if let Some(cancellation) = &options.cancellation {
        cancellation.cancel();
    }
}

pub(crate) fn validate_cached(
    scanned: &mut ScannedFile,
    expected_fingerprint: &str,
    options: &ScanOptions,
    started: Instant,
) -> io::Result<CachedValidation> {
    let mut file = File::open(&scanned.absolute)?;
    let before = snapshot(&file)?;
    if before.bytes != scanned.bytes || !reusable(&scanned.version, &before.version) {
        return Ok(CachedValidation::Concurrent);
    }

    let mut fingerprint = ContentFingerprint::new();
    let mut buffer = vec![0_u8; 64 * 1024].into_boxed_slice();
    let (bytes_read, status) = crate::content::bounded::read_bounded(
        &mut file,
        &mut buffer,
        super::read_limits(before.bytes, options),
        super::read_control(options, started),
        |chunk| {
            fingerprint.write(chunk);
            Ok(true)
        },
    )?;
    if !matches!(status, crate::content::bounded::BoundedReadStatus::Complete)
        || bytes_read != before.bytes
    {
        return Ok(CachedValidation::Concurrent);
    }

    let after = snapshot(&file)?;
    if before.bytes != after.bytes || !reusable(&before.version, &after.version) {
        return Ok(CachedValidation::Concurrent);
    }
    scanned.version = after.version;
    if fingerprint.finish() == expected_fingerprint {
        Ok(CachedValidation::Match)
    } else {
        Ok(CachedValidation::Changed)
    }
}
