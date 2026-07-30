use super::*;

pub(crate) fn inspect(mut scanned: ScannedFile, options: &ScanOptions) -> io::Result<Inspection> {
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

    let after = if options.content_validation == ContentValidationPolicy::Strict {
        Some(snapshot(&file)?)
    } else {
        None
    };
    if (reads_entire_file && bytes_read != before.bytes)
        || after.as_ref().is_some_and(|after| {
            before.bytes != after.bytes || !reusable(&before.version, &after.version)
        })
    {
        return Ok(Inspection::Concurrent(scanned.relative));
    }
    if binary {
        return Ok(Inspection::Binary(scanned.relative));
    }
    scanned.version = after.map_or(before.version, |after| after.version);
    scanned.binary_checked = options.detect_binary_files;
    scanned.content_hash = hasher.map(FingerprintHasher::finish);
    scanned.content_fingerprint = fingerprint.map(ContentFingerprint::finish);
    Ok(Inspection::Selected(scanned))
}
