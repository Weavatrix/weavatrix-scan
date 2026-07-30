use super::{
    CompactContentEvidence, CompactScannedFile, ContentVisitControl, ContentVisitEvent,
    ContentVisitMode, ContentWorkerContext, EvidenceMode, InspectionStop, Instant, Result,
    ScanOptions, ScanTermination, ScannedFile, SkipKind, SkippedEntry, VisitedFiles, VisitedStatus,
    inspect, record_binary_skip, record_concurrent_modification, record_io_error,
    record_limit_skip,
};

pub(crate) fn visit_files<V, I>(
    files: I,
    options: &ScanOptions,
    started: Instant,
    context: ContentWorkerContext<'_>,
    buffer: &mut [u8],
    visitor: &mut V,
) -> Result<VisitedFiles>
where
    V: for<'event> FnMut(ContentVisitEvent<'event>) -> ContentVisitControl,
    I: IntoIterator<Item = (u64, CompactScannedFile)>,
{
    let stop = InspectionStop::default();
    let mut iterator = files.into_iter();
    let mut visited = VisitedFiles::empty(iterator.size_hint().0);
    while let Some((sequence, compact)) = iterator.next() {
        if let Some(reason) = stop.reason(options, started) {
            record_limit_skip(
                &mut visited.evidence,
                compact.relative.into(),
                reason,
                options,
            );
            if options.evidence == EvidenceMode::Complete {
                visited
                    .evidence
                    .skipped
                    .extend(iterator.map(|(_, file)| SkippedEntry {
                        relative: file.relative.into(),
                        kind: SkipKind::ScanLimit,
                        detail: Some(format!("{reason:?}")),
                    }));
            }
            visited.evidence.termination = Some(reason);
            break;
        }
        if visit_file(
            sequence,
            compact,
            options,
            context,
            buffer,
            visitor,
            &mut visited,
        )? {
            break;
        }
    }
    Ok(visited)
}

#[allow(clippy::too_many_arguments)]
fn visit_file<V>(
    sequence: u64,
    mut compact: CompactScannedFile,
    options: &ScanOptions,
    context: ContentWorkerContext<'_>,
    buffer: &mut [u8],
    visitor: &mut V,
    visited: &mut VisitedFiles,
) -> Result<bool>
where
    V: for<'event> FnMut(ContentVisitEvent<'event>) -> ContentVisitControl,
{
    let discovery = compact
        .content
        .take()
        .expect("content visit retains file-version evidence");
    let relative = compact.relative.into_string();
    let mut scanned = ScannedFile {
        absolute: context.root.join(&relative),
        relative,
        bytes: compact.bytes,
        content_hash: None,
        content_fingerprint: None,
        version: discovery.version,
        binary_checked: false,
    };
    visited.evidence.cache.content_reads = visited.evidence.cache.content_reads.saturating_add(1);
    match inspect::inspect_with_visitor(&mut scanned, options, context, sequence, buffer, visitor) {
        Ok(result) => record_visit_result(visited, scanned, &result, options, context, sequence),
        Err(source) => {
            record_io_error(
                &mut visited.evidence,
                &scanned,
                "visit file content",
                source,
                options,
            )?;
            Ok(false)
        }
    }
}

fn record_visit_result(
    visited: &mut VisitedFiles,
    scanned: ScannedFile,
    result: &inspect::VisitedInspection,
    options: &ScanOptions,
    context: ContentWorkerContext<'_>,
    sequence: u64,
) -> Result<bool> {
    let cancelled_without_result = result.status.is_none()
        && options
            .cancellation
            .as_ref()
            .is_some_and(crate::control::CancellationToken::is_cancelled);
    visited.opened = visited.opened.saturating_add(result.opened);
    visited.chunks = visited.chunks.saturating_add(result.chunks);
    visited.bytes_read = visited.bytes_read.saturating_add(result.bytes_read);
    visited.bytes_emitted = visited.bytes_emitted.saturating_add(result.bytes_emitted);
    if result.consumer_skipped {
        visited.consumer_skipped = visited.consumer_skipped.saturating_add(1);
    }
    match result.status.as_ref() {
        Some(VisitedStatus::Selected) => {
            record_selected(visited, scanned, options, context, sequence);
        }
        Some(VisitedStatus::Binary) => {
            record_binary_skip(&mut visited.evidence, scanned.relative, options);
        }
        Some(VisitedStatus::Concurrent) => {
            record_concurrent_modification(&mut visited.evidence, scanned.relative, options)?;
        }
        None => {}
    }
    if result.visitor_quit {
        visited.visitor_quit = true;
        visited.evidence.termination = Some(ScanTermination::Cancelled);
        return Ok(true);
    }
    if cancelled_without_result {
        visited.evidence.termination = Some(ScanTermination::Cancelled);
        return Ok(true);
    }
    Ok(false)
}

fn record_selected(
    visited: &mut VisitedFiles,
    scanned: ScannedFile,
    options: &ScanOptions,
    context: ContentWorkerContext<'_>,
    sequence: u64,
) {
    visited.completed = visited.completed.saturating_add(1);
    if context.mode != ContentVisitMode::Revision {
        return;
    }
    visited.files.push((
        sequence,
        CompactScannedFile {
            relative: scanned.relative.into_boxed_str(),
            bytes: scanned.bytes,
            content: (options.hash_file_contents || options.detect_binary_files).then(|| {
                Box::new(CompactContentEvidence {
                    content_hash: scanned.content_hash.map(String::into_boxed_str),
                    content_fingerprint: scanned.content_fingerprint.map(String::into_boxed_str),
                    version: scanned.version,
                    binary_checked: scanned.binary_checked,
                })
            }),
        },
    ));
}
