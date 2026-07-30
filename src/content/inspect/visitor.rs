use super::support::{cancel, content_file, emit_end};
use super::*;

struct VisitProgress {
    hasher: Option<FingerprintHasher>,
    fingerprint: Option<ContentFingerprint>,
    binary: bool,
    bytes_read: u64,
    bytes_emitted: u64,
    chunks: u64,
    consumer_skipped: bool,
    reached_eof: bool,
}

impl VisitProgress {
    fn new(options: &ScanOptions) -> Self {
        Self {
            hasher: options.hash_file_contents.then(FingerprintHasher::new),
            fingerprint: options.hash_file_contents.then(ContentFingerprint::new),
            binary: false,
            bytes_read: 0,
            bytes_emitted: 0,
            chunks: 0,
            consumer_skipped: false,
            reached_eof: false,
        }
    }

    const fn outcome(
        &self,
        status: Option<VisitedStatus>,
        visitor_quit: bool,
    ) -> VisitedInspection {
        VisitedInspection {
            status,
            opened: 1,
            chunks: self.chunks,
            bytes_read: self.bytes_read,
            bytes_emitted: self.bytes_emitted,
            consumer_skipped: self.consumer_skipped,
            visitor_quit,
        }
    }
}

pub(crate) fn inspect_with_visitor<V>(
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
    if before.bytes != scanned.bytes
        || !reusable(&scanned.version, &before.version)
        || before.bytes > options.max_file_bytes
    {
        return Ok(concurrent_outcome());
    }

    let mut progress = VisitProgress::new(options);
    match visitor(ContentVisitEvent::FileStart {
        worker_index: context.worker_index,
        file: content_file(context.root, context.root_index, scanned, sequence),
    }) {
        ContentVisitControl::Continue => {}
        ContentVisitControl::SkipFile => progress.consumer_skipped = true,
        ContentVisitControl::Quit => {
            cancel(options);
            return Ok(progress.outcome(None, true));
        }
    }

    if let Some(visitor_quit) = read_chunks(
        &mut file,
        scanned,
        options,
        context,
        sequence,
        buffer,
        visitor,
        &mut progress,
    )? {
        return Ok(progress.outcome(None, visitor_quit));
    }
    finish_visit(
        &file, scanned, options, context, sequence, visitor, &before, progress,
    )
}

const fn concurrent_outcome() -> VisitedInspection {
    VisitedInspection {
        status: Some(VisitedStatus::Concurrent),
        opened: 1,
        chunks: 0,
        bytes_read: 0,
        bytes_emitted: 0,
        consumer_skipped: false,
        visitor_quit: false,
    }
}

#[allow(clippy::too_many_arguments)]
fn read_chunks<V>(
    file: &mut File,
    scanned: &ScannedFile,
    options: &ScanOptions,
    context: ContentWorkerContext<'_>,
    sequence: u64,
    buffer: &mut [u8],
    visitor: &mut V,
    progress: &mut VisitProgress,
) -> io::Result<Option<bool>>
where
    V: for<'event> FnMut(ContentVisitEvent<'event>) -> ContentVisitControl,
{
    while !progress.consumer_skipped || progress.hasher.is_some() || options.detect_binary_files {
        if options
            .cancellation
            .as_ref()
            .is_some_and(crate::control::CancellationToken::is_cancelled)
        {
            return Ok(Some(false));
        }
        let read = file.read(buffer)?;
        if read == 0 {
            progress.reached_eof = true;
            break;
        }
        let offset = progress.bytes_read;
        progress.bytes_read = progress.bytes_read.saturating_add(read as u64);
        let bytes = &buffer[..read];
        if options.detect_binary_files && bytes.contains(&0) {
            progress.binary = true;
            break;
        }
        if let Some(hasher) = progress.hasher.as_mut() {
            hasher.write(bytes);
        }
        if let Some(fingerprint) = progress.fingerprint.as_mut() {
            fingerprint.write(bytes);
        }
        if progress.consumer_skipped {
            continue;
        }
        progress.chunks = progress.chunks.saturating_add(1);
        progress.bytes_emitted = progress.bytes_emitted.saturating_add(read as u64);
        match visitor(ContentVisitEvent::Chunk {
            worker_index: context.worker_index,
            file: content_file(context.root, context.root_index, scanned, sequence),
            offset,
            bytes,
        }) {
            ContentVisitControl::Continue => {}
            ContentVisitControl::SkipFile => progress.consumer_skipped = true,
            ContentVisitControl::Quit => {
                cancel(options);
                return Ok(Some(true));
            }
        }
    }
    Ok(None)
}

#[allow(clippy::too_many_arguments)]
fn finish_visit<V>(
    file: &File,
    scanned: &mut ScannedFile,
    options: &ScanOptions,
    context: ContentWorkerContext<'_>,
    sequence: u64,
    visitor: &mut V,
    before: &crate::file_version::FileSnapshot,
    mut progress: VisitProgress,
) -> io::Result<VisitedInspection>
where
    V: for<'event> FnMut(ContentVisitEvent<'event>) -> ContentVisitControl,
{
    let after = if options.content_validation == ContentValidationPolicy::Strict {
        Some(snapshot(file)?)
    } else {
        None
    };
    if (progress.reached_eof && progress.bytes_read != before.bytes)
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
            progress.bytes_read,
            None,
            progress.consumer_skipped,
            options,
        );
        return Ok(progress.outcome(Some(VisitedStatus::Concurrent), visitor_quit));
    }
    if progress.binary {
        let visitor_quit = emit_end(
            visitor,
            context.worker_index,
            context.root,
            context.root_index,
            scanned,
            sequence,
            ContentFileStatus::Binary,
            progress.bytes_read,
            None,
            progress.consumer_skipped,
            options,
        );
        return Ok(progress.outcome(Some(VisitedStatus::Binary), visitor_quit));
    }

    scanned.version = after.map_or(before.version, |after| after.version);
    scanned.binary_checked = options.detect_binary_files;
    scanned.content_hash = progress.hasher.take().map(FingerprintHasher::finish);
    scanned.content_fingerprint = progress.fingerprint.take().map(ContentFingerprint::finish);
    let visitor_quit = emit_end(
        visitor,
        context.worker_index,
        context.root,
        context.root_index,
        scanned,
        sequence,
        ContentFileStatus::Selected,
        progress.bytes_read,
        scanned.content_hash.as_deref(),
        progress.consumer_skipped,
        options,
    );
    Ok(progress.outcome(Some(VisitedStatus::Selected), visitor_quit))
}
