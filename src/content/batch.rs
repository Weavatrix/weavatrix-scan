use super::{
    CacheValidationPolicy, HashMap, InspectedFiles, Inspection, InspectionStop, Instant, Result,
    ScanCache, ScanCacheEntry, ScanCacheStats, ScanOptions, ScannedFile, apply_cached, inspect,
    record_binary_skip, record_concurrent_modification, record_io_error, record_limit_skip,
    reusable_candidate,
};

pub(crate) fn inspect_files(
    files: Vec<ScannedFile>,
    options: &ScanOptions,
    started: Instant,
    previous: Option<&ScanCache>,
) -> Result<InspectedFiles> {
    if !options.hash_file_contents && !options.detect_binary_files {
        return Ok(InspectedFiles {
            files,
            skipped: Vec::new(),
            warnings: Vec::new(),
            termination: None,
            cache: ScanCacheStats::default(),
        });
    }
    let cache = previous.map_or_else(HashMap::new, |cache| {
        cache
            .entries
            .iter()
            .map(|entry| (entry.relative.as_str(), entry))
            .collect()
    });
    let stop = InspectionStop::default();
    let workers = options.worker_count(files.len());
    if workers <= 1 {
        return inspect_chunk(files, options, started, &stop, &cache);
    }
    let chunk_size = files.len().div_ceil(workers);
    let mut iterator = files.into_iter();
    let mut chunks = Vec::with_capacity(workers);
    loop {
        let chunk = iterator.by_ref().take(chunk_size).collect::<Vec<_>>();
        if chunk.is_empty() {
            break;
        }
        chunks.push(chunk);
    }

    std::thread::scope(|scope| {
        let handles = chunks
            .into_iter()
            .map(|chunk| scope.spawn(|| inspect_chunk(chunk, options, started, &stop, &cache)))
            .collect::<Vec<_>>();
        let mut inspected = InspectedFiles {
            files: Vec::new(),
            skipped: Vec::new(),
            warnings: Vec::new(),
            termination: None,
            cache: ScanCacheStats::default(),
        };
        for handle in handles {
            let chunk = handle.join().expect("content inspection worker panicked")?;
            inspected.files.extend(chunk.files);
            inspected.skipped.extend(chunk.skipped);
            inspected.warnings.extend(chunk.warnings);
            inspected.termination = inspected.termination.or(chunk.termination);
            inspected.cache.reused_hashes = inspected
                .cache
                .reused_hashes
                .saturating_add(chunk.cache.reused_hashes);
            inspected.cache.content_reads = inspected
                .cache
                .content_reads
                .saturating_add(chunk.cache.content_reads);
            inspected.cache.fingerprint_reads = inspected
                .cache
                .fingerprint_reads
                .saturating_add(chunk.cache.fingerprint_reads);
        }
        Ok(inspected)
    })
}

fn inspect_chunk(
    files: Vec<ScannedFile>,
    options: &ScanOptions,
    started: Instant,
    stop: &InspectionStop,
    cache: &HashMap<&str, &ScanCacheEntry>,
) -> Result<InspectedFiles> {
    let mut inspected = InspectedFiles {
        files: Vec::with_capacity(files.len()),
        skipped: Vec::new(),
        warnings: Vec::new(),
        termination: None,
        cache: ScanCacheStats::default(),
    };
    for mut file in files {
        if let Some(reason) = stop.reason(options, started) {
            record_limit_skip(&mut inspected, file.relative, reason, options);
            inspected.termination = Some(reason);
            continue;
        }
        let cached = cache.get(file.relative.as_str()).copied();
        if reusable_candidate(&file, options, cached) {
            let cached = cached.expect("reusable candidate has cache evidence");
            if options.cache_validation == CacheValidationPolicy::Fast {
                apply_cached(&mut file, cached);
                inspected.cache.reused_hashes = inspected.cache.reused_hashes.saturating_add(1);
                inspected.files.push(file);
                continue;
            }
            inspected.cache.content_reads = inspected.cache.content_reads.saturating_add(1);
            inspected.cache.fingerprint_reads = inspected.cache.fingerprint_reads.saturating_add(1);
            match inspect::validate_cached(&mut file, &cached.content_fingerprint) {
                Ok(inspect::CachedValidation::Match) => {
                    apply_cached(&mut file, cached);
                    inspected.cache.reused_hashes = inspected.cache.reused_hashes.saturating_add(1);
                    inspected.files.push(file);
                    continue;
                }
                Ok(inspect::CachedValidation::Changed) => {}
                Ok(inspect::CachedValidation::Concurrent) => {
                    record_concurrent_modification(&mut inspected, file.relative, options)?;
                    continue;
                }
                Err(source) => {
                    record_io_error(
                        &mut inspected,
                        &file,
                        "validate cached content",
                        source,
                        options,
                    )?;
                    continue;
                }
            }
        }
        inspected.cache.content_reads = inspected.cache.content_reads.saturating_add(1);
        let error_file = file.clone();
        match inspect::inspect(file, options) {
            Ok(Inspection::Selected(file)) => inspected.files.push(file),
            Ok(Inspection::Binary(relative)) => {
                record_binary_skip(&mut inspected, relative, options);
            }
            Ok(Inspection::Concurrent(relative)) => {
                record_concurrent_modification(&mut inspected, relative, options)?;
            }
            Err(source) => {
                record_io_error(
                    &mut inspected,
                    &error_file,
                    "inspect file content",
                    source,
                    options,
                )?;
            }
        }
    }
    Ok(inspected)
}
