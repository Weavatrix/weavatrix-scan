use crate::config::ScanOptions;
use crate::file_version::{from_file, reusable};
use crate::hash::{ContentFingerprint, FingerprintHasher};
use crate::report::ScannedFile;
use std::fs::File;
use std::io::{self, Read as _};

pub(super) enum Inspection {
    Selected(ScannedFile),
    Binary(String),
    Concurrent(String),
}

pub(super) enum CachedValidation {
    Match,
    Changed,
    Concurrent,
}

pub(super) fn inspect(mut scanned: ScannedFile, options: &ScanOptions) -> io::Result<Inspection> {
    let mut file = File::open(&scanned.absolute)?;
    let before_metadata = file.metadata()?;
    let before_version = from_file(&file, &before_metadata)?;
    if before_metadata.len() != scanned.bytes || !reusable(&scanned.version, &before_version) {
        return Ok(Inspection::Concurrent(scanned.relative));
    }
    if before_metadata.len() > options.max_file_bytes {
        return Ok(Inspection::Concurrent(scanned.relative));
    }

    let mut hasher = options.hash_file_contents.then(FingerprintHasher::new);
    let mut fingerprint = options.hash_file_contents.then(ContentFingerprint::new);
    let reads_entire_file = hasher.is_some();
    let mut binary = false;
    let mut bytes_read = 0_u64;
    let mut buffer = vec![0_u8; 64 * 1024].into_boxed_slice();
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        bytes_read = bytes_read.saturating_add(read as u64);
        if options.detect_binary_files && buffer[..read].contains(&0) {
            binary = true;
        }
        if let Some(hasher) = hasher.as_mut() {
            hasher.write(&buffer[..read]);
        } else if bytes_read >= 8 * 1024 {
            break;
        }
        if let Some(fingerprint) = fingerprint.as_mut() {
            fingerprint.write(&buffer[..read]);
        }
    }

    let after_metadata = file.metadata()?;
    let after_version = from_file(&file, &after_metadata)?;
    if (reads_entire_file && bytes_read != before_metadata.len())
        || before_metadata.len() != after_metadata.len()
        || !reusable(&before_version, &after_version)
    {
        return Ok(Inspection::Concurrent(scanned.relative));
    }
    if binary {
        return Ok(Inspection::Binary(scanned.relative));
    }
    scanned.version = after_version;
    scanned.binary_checked = options.detect_binary_files;
    scanned.content_hash = hasher.map(FingerprintHasher::finish);
    scanned.content_fingerprint = fingerprint.map(ContentFingerprint::finish);
    Ok(Inspection::Selected(scanned))
}

pub(super) fn validate_cached(
    scanned: &mut ScannedFile,
    expected_fingerprint: &str,
) -> io::Result<CachedValidation> {
    let mut file = File::open(&scanned.absolute)?;
    let before_metadata = file.metadata()?;
    let before_version = from_file(&file, &before_metadata)?;
    if before_metadata.len() != scanned.bytes || !reusable(&scanned.version, &before_version) {
        return Ok(CachedValidation::Concurrent);
    }

    let mut fingerprint = ContentFingerprint::new();
    let mut bytes_read = 0_u64;
    let mut buffer = vec![0_u8; 64 * 1024].into_boxed_slice();
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        bytes_read = bytes_read.saturating_add(read as u64);
        fingerprint.write(&buffer[..read]);
    }

    let after_metadata = file.metadata()?;
    let after_version = from_file(&file, &after_metadata)?;
    if bytes_read != before_metadata.len()
        || before_metadata.len() != after_metadata.len()
        || !reusable(&before_version, &after_version)
    {
        return Ok(CachedValidation::Concurrent);
    }
    scanned.version = after_version;
    if fingerprint.finish() == expected_fingerprint {
        Ok(CachedValidation::Match)
    } else {
        Ok(CachedValidation::Changed)
    }
}
