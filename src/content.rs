use crate::config::{EvidenceMode, ScanOptions};
use crate::error::{Error, Result};
use crate::path::{RevisionHasher, looks_binary};
use crate::report::{ScanTermination, ScanWarning, ScannedFile, SkipKind, SkippedEntry};
use crate::walker::ErrorPolicy;
use std::fs;
use std::io;
use std::io::Read as _;
use std::path::Path;
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::Instant;

pub(crate) struct InspectedFiles {
    pub(crate) files: Vec<ScannedFile>,
    pub(crate) skipped: Vec<SkippedEntry>,
    pub(crate) warnings: Vec<ScanWarning>,
    pub(crate) termination: Option<ScanTermination>,
}

pub(crate) fn inspect_files(
    files: Vec<ScannedFile>,
    options: &ScanOptions,
    started: Instant,
) -> Result<InspectedFiles> {
    if !options.hash_file_contents && !options.detect_binary_files {
        return Ok(InspectedFiles {
            files,
            skipped: Vec::new(),
            warnings: Vec::new(),
            termination: None,
        });
    }
    let stop = InspectionStop::default();
    let workers = options.worker_count(files.len());
    if workers <= 1 {
        return inspect_chunk(files, options, started, &stop);
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
            .map(|chunk| scope.spawn(|| inspect_chunk(chunk, options, started, &stop)))
            .collect::<Vec<_>>();
        let mut inspected = InspectedFiles {
            files: Vec::new(),
            skipped: Vec::new(),
            warnings: Vec::new(),
            termination: None,
        };
        for handle in handles {
            let chunk = handle.join().expect("content inspection worker panicked")?;
            inspected.files.extend(chunk.files);
            inspected.skipped.extend(chunk.skipped);
            inspected.warnings.extend(chunk.warnings);
            inspected.termination = inspected.termination.or(chunk.termination);
        }
        Ok(inspected)
    })
}

fn inspect_chunk(
    files: Vec<ScannedFile>,
    options: &ScanOptions,
    started: Instant,
    stop: &InspectionStop,
) -> Result<InspectedFiles> {
    let mut inspected = InspectedFiles {
        files: Vec::with_capacity(files.len()),
        skipped: Vec::new(),
        warnings: Vec::new(),
        termination: None,
    };
    for mut file in files {
        if let Some(reason) = stop.reason(options, started) {
            record_limit_skip(&mut inspected, file.relative, reason, options);
            inspected.termination = Some(reason);
            continue;
        }
        if options.hash_file_contents {
            let bytes = match fs::read(&file.absolute) {
                Ok(bytes) => bytes,
                Err(source) => {
                    record_io_error(&mut inspected, &file, "read file content", source, options)?;
                    continue;
                }
            };
            if options.detect_binary_files && looks_binary(&bytes) {
                record_binary_skip(&mut inspected, file.relative, options);
                continue;
            }
            file.content_hash = Some(hash_bytes(&bytes));
        } else if options.detect_binary_files {
            match file_looks_binary(&file.absolute) {
                Ok(true) => {
                    record_binary_skip(&mut inspected, file.relative, options);
                    continue;
                }
                Ok(false) => {}
                Err(source) => {
                    record_io_error(
                        &mut inspected,
                        &file,
                        "inspect file content",
                        source,
                        options,
                    )?;
                    continue;
                }
            }
        }
        inspected.files.push(file);
    }
    Ok(inspected)
}

#[derive(Default)]
struct InspectionStop(AtomicU8);

impl InspectionStop {
    fn reason(&self, options: &ScanOptions, started: Instant) -> Option<ScanTermination> {
        if let Some(reason) = termination_from_code(self.0.load(Ordering::Acquire)) {
            return Some(reason);
        }
        let reason = if options
            .cancellation
            .as_ref()
            .is_some_and(crate::CancellationToken::is_cancelled)
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

const fn termination_code(reason: ScanTermination) -> u8 {
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

fn record_limit_skip(
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

fn binary_skip(relative: String) -> SkippedEntry {
    SkippedEntry {
        relative,
        kind: SkipKind::Binary,
        detail: None,
    }
}

fn record_binary_skip(inspected: &mut InspectedFiles, relative: String, options: &ScanOptions) {
    if options.evidence == EvidenceMode::Complete {
        inspected.skipped.push(binary_skip(relative));
    }
}

fn hash_bytes(bytes: &[u8]) -> String {
    let mut hasher = RevisionHasher::new();
    hasher.write(bytes);
    hasher.finish()
}

fn file_looks_binary(path: &Path) -> io::Result<bool> {
    let mut file = fs::File::open(path)?;
    let mut buffer = [0; 8192];
    let read = file.read(&mut buffer)?;
    Ok(looks_binary(&buffer[..read]))
}

fn record_io_error(
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

#[cfg(test)]
#[path = "content/tests.rs"]
mod tests;
