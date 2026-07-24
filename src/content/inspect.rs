use crate::config::{ContentValidationPolicy, ScanOptions};
use crate::content_visit::{
    ContentFile, ContentFileStatus, ContentVisitControl, ContentVisitEvent,
};
use crate::file_version::{reusable, snapshot};
use crate::hash::{ContentFingerprint, FingerprintHasher};
use crate::report::ScannedFile;
use std::fs::File;
use std::io::{self, Read as _};
use std::path::Path;

use super::ContentWorkerContext;

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

pub(super) struct VisitedInspection {
    pub status: Option<VisitedStatus>,
    pub opened: u64,
    pub chunks: u64,
    pub bytes_read: u64,
    pub bytes_emitted: u64,
    pub consumer_skipped: bool,
    pub visitor_quit: bool,
}

pub(super) enum VisitedStatus {
    Selected,
    Binary,
    Concurrent,
}

pub(super) fn inspect(mut scanned: ScannedFile, options: &ScanOptions) -> io::Result<Inspection> {
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

#[allow(clippy::too_many_lines)]
pub(super) fn inspect_with_visitor<V>(
    scanned: &mut ScannedFile,
    options: &ScanOptions,
    context: ContentWorkerContext<'_>,
    sequence: u64,
    buffer: &mut [u8],
    visitor: &mut V,
) -> io::Result<VisitedInspection>
where
    V: for<'event> FnMut(ContentVisitEvent<'event>) -> ContentVisitControl,
{
    let mut file = File::open(&scanned.absolute)?;
    let before = snapshot(&file)?;
    if before.bytes != scanned.bytes || !reusable(&scanned.version, &before.version) {
        return Ok(VisitedInspection {
            status: Some(VisitedStatus::Concurrent),
            opened: 1,
            chunks: 0,
            bytes_read: 0,
            bytes_emitted: 0,
            consumer_skipped: false,
            visitor_quit: false,
        });
    }
    if before.bytes > options.max_file_bytes {
        return Ok(VisitedInspection {
            status: Some(VisitedStatus::Concurrent),
            opened: 1,
            chunks: 0,
            bytes_read: 0,
            bytes_emitted: 0,
            consumer_skipped: false,
            visitor_quit: false,
        });
    }

    let mut consumer_skipped = false;
    match visitor(ContentVisitEvent::FileStart {
        worker_index: context.worker_index,
        file: content_file(context.root, context.root_index, scanned, sequence),
    }) {
        ContentVisitControl::Continue => {}
        ContentVisitControl::SkipFile => consumer_skipped = true,
        ContentVisitControl::Quit => {
            cancel(options);
            return Ok(VisitedInspection {
                status: None,
                opened: 1,
                chunks: 0,
                bytes_read: 0,
                bytes_emitted: 0,
                consumer_skipped: false,
                visitor_quit: true,
            });
        }
    }

    let mut hasher = options.hash_file_contents.then(FingerprintHasher::new);
    let mut fingerprint = options.hash_file_contents.then(ContentFingerprint::new);
    let mut binary = false;
    let mut bytes_read = 0_u64;
    let mut bytes_emitted = 0_u64;
    let mut chunks = 0_u64;
    let mut reached_eof = false;
    while !consumer_skipped || hasher.is_some() || options.detect_binary_files {
        if options
            .cancellation
            .as_ref()
            .is_some_and(crate::CancellationToken::is_cancelled)
        {
            return Ok(VisitedInspection {
                status: None,
                opened: 1,
                chunks,
                bytes_read,
                bytes_emitted,
                consumer_skipped,
                visitor_quit: false,
            });
        }
        let read = file.read(buffer)?;
        if read == 0 {
            reached_eof = true;
            break;
        }
        let offset = bytes_read;
        bytes_read = bytes_read.saturating_add(read as u64);
        let bytes = &buffer[..read];
        if options.detect_binary_files && bytes.contains(&0) {
            binary = true;
            break;
        }
        if let Some(hasher) = hasher.as_mut() {
            hasher.write(bytes);
        }
        if let Some(fingerprint) = fingerprint.as_mut() {
            fingerprint.write(bytes);
        }
        if !consumer_skipped {
            chunks = chunks.saturating_add(1);
            bytes_emitted = bytes_emitted.saturating_add(read as u64);
            match visitor(ContentVisitEvent::Chunk {
                worker_index: context.worker_index,
                file: content_file(context.root, context.root_index, scanned, sequence),
                offset,
                bytes,
            }) {
                ContentVisitControl::Continue => {}
                ContentVisitControl::SkipFile => consumer_skipped = true,
                ContentVisitControl::Quit => {
                    cancel(options);
                    return Ok(VisitedInspection {
                        status: None,
                        opened: 1,
                        chunks,
                        bytes_read,
                        bytes_emitted,
                        consumer_skipped,
                        visitor_quit: true,
                    });
                }
            }
        }
    }

    let after = if options.content_validation == ContentValidationPolicy::Strict {
        Some(snapshot(&file)?)
    } else {
        None
    };
    if (reached_eof && bytes_read != before.bytes)
        || after.as_ref().is_some_and(|after| {
            before.bytes != after.bytes || !reusable(&before.version, &after.version)
        })
    {
        let visitor_quit = emit_end(
            visitor,
            context.worker_index,
            context.root,
            context.root_index,
            scanned,
            sequence,
            ContentFileStatus::Changed,
            bytes_read,
            None,
            consumer_skipped,
            options,
        );
        return Ok(VisitedInspection {
            status: Some(VisitedStatus::Concurrent),
            opened: 1,
            chunks,
            bytes_read,
            bytes_emitted,
            consumer_skipped,
            visitor_quit,
        });
    }
    if binary {
        let visitor_quit = emit_end(
            visitor,
            context.worker_index,
            context.root,
            context.root_index,
            scanned,
            sequence,
            ContentFileStatus::Binary,
            bytes_read,
            None,
            consumer_skipped,
            options,
        );
        return Ok(VisitedInspection {
            status: Some(VisitedStatus::Binary),
            opened: 1,
            chunks,
            bytes_read,
            bytes_emitted,
            consumer_skipped,
            visitor_quit,
        });
    }

    scanned.version = after.map_or(before.version, |after| after.version);
    scanned.binary_checked = options.detect_binary_files;
    scanned.content_hash = hasher.map(FingerprintHasher::finish);
    scanned.content_fingerprint = fingerprint.map(ContentFingerprint::finish);
    let visitor_quit = emit_end(
        visitor,
        context.worker_index,
        context.root,
        context.root_index,
        scanned,
        sequence,
        ContentFileStatus::Selected,
        bytes_read,
        scanned.content_hash.as_deref(),
        consumer_skipped,
        options,
    );
    Ok(VisitedInspection {
        status: Some(VisitedStatus::Selected),
        opened: 1,
        chunks,
        bytes_read,
        bytes_emitted,
        consumer_skipped,
        visitor_quit,
    })
}

fn content_file<'a>(
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
fn emit_end<V>(
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

fn cancel(options: &ScanOptions) {
    if let Some(cancellation) = &options.cancellation {
        cancellation.cancel();
    }
}

pub(super) fn validate_cached(
    scanned: &mut ScannedFile,
    expected_fingerprint: &str,
) -> io::Result<CachedValidation> {
    let mut file = File::open(&scanned.absolute)?;
    let before = snapshot(&file)?;
    if before.bytes != scanned.bytes || !reusable(&scanned.version, &before.version) {
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

    let after = snapshot(&file)?;
    if bytes_read != before.bytes
        || before.bytes != after.bytes
        || !reusable(&before.version, &after.version)
    {
        return Ok(CachedValidation::Concurrent);
    }
    scanned.version = after.version;
    if fingerprint.finish() == expected_fingerprint {
        Ok(CachedValidation::Match)
    } else {
        Ok(CachedValidation::Changed)
    }
}
