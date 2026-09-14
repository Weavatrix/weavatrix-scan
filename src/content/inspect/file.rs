#![allow(clippy::wildcard_imports)]

use super::*;
use crate::content::bounded::{BoundedReadStatus, read_bounded};

pub(crate) fn inspect(
    mut scanned: ScannedFile,
    options: &ScanOptions,
    started: Instant,
) -> io::Result<Inspection> {
    let mut file = File::open(&scanned.absolute)?;
    let before = snapshot(&file)?;
    if before.bytes != scanned.bytes || !reusable(&scanned.version, &before.version) {
        return Ok(Inspection::Concurrent(scanned.relative));
    }
    if before.bytes > options.max_file_bytes {
        return Ok(Inspection::Concurrent(scanned.relative));
    }

    let mut hasher = options.hash_file_contents.then(FingerprintHasher::new);
    let mut fingerprint = options.hash_file_contents.then(ContentFingerprint::new);
    let reads_entire_file = hasher.is_some();
    let mut binary = false;
    let mut consumed = 0_u64;
    let mut buffer = vec![0_u8; 64 * 1024].into_boxed_slice();
    let (bytes_read, status) = read_bounded(
        &mut file,
        &mut buffer,
        read_limits(before.bytes, options),
        read_control(options, started),
        |chunk| {
            if options.detect_binary_files && chunk.contains(&0) {
                binary = true;
                return Ok(false);
            }
            if let Some(hasher) = hasher.as_mut() {
                hasher.write(chunk);
            }
            if let Some(fingerprint) = fingerprint.as_mut() {
                fingerprint.write(chunk);
            }
            consumed = consumed.saturating_add(chunk.len() as u64);
            Ok(reads_entire_file || consumed < 8 * 1024)
        },
    )?;
    if binary {
        return Ok(Inspection::Binary(scanned.relative));
    }
    if matches!(
        status,
        BoundedReadStatus::Grown | BoundedReadStatus::Cancelled | BoundedReadStatus::Deadline
    ) || (reads_entire_file
        && (status != BoundedReadStatus::Complete || bytes_read != before.bytes))
    {
        return Ok(Inspection::Concurrent(scanned.relative));
    }

    let after = if options.content_validation == ContentValidationPolicy::Strict {
        Some(snapshot(&file)?)
    } else {
        None
    };
    if after.as_ref().is_some_and(|after| {
        before.bytes != after.bytes || !reusable(&before.version, &after.version)
    }) {
        return Ok(Inspection::Concurrent(scanned.relative));
    }
    scanned.version = after.map_or(before.version, |after| after.version);
    scanned.binary_checked = options.detect_binary_files;
    scanned.content_hash = hasher.map(FingerprintHasher::finish);
    scanned.content_fingerprint = fingerprint.map(ContentFingerprint::finish);
    Ok(Inspection::Selected(scanned))
}
