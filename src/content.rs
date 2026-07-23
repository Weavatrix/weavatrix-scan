use crate::config::ScanOptions;
use crate::error::{Error, Result};
use crate::path::{RevisionHasher, looks_binary};
use crate::report::{ScannedFile, SkipKind, SkippedEntry};
use std::fs;
use std::io::Read as _;
use std::path::Path;

pub(crate) struct InspectedFiles {
    pub(crate) files: Vec<ScannedFile>,
    pub(crate) skipped: Vec<SkippedEntry>,
}

pub(crate) fn inspect_files(
    files: Vec<ScannedFile>,
    options: &ScanOptions,
) -> Result<InspectedFiles> {
    if !options.hash_file_contents && !options.detect_binary_files {
        return Ok(InspectedFiles {
            files,
            skipped: Vec::new(),
        });
    }
    let workers = options.worker_count(files.len());
    if workers <= 1 {
        return inspect_chunk(files, options);
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
            .map(|chunk| scope.spawn(move || inspect_chunk(chunk, options)))
            .collect::<Vec<_>>();
        let mut inspected = InspectedFiles {
            files: Vec::new(),
            skipped: Vec::new(),
        };
        for handle in handles {
            let chunk = handle.join().expect("content inspection worker panicked")?;
            inspected.files.extend(chunk.files);
            inspected.skipped.extend(chunk.skipped);
        }
        Ok(inspected)
    })
}

fn inspect_chunk(files: Vec<ScannedFile>, options: &ScanOptions) -> Result<InspectedFiles> {
    let mut inspected = InspectedFiles {
        files: Vec::with_capacity(files.len()),
        skipped: Vec::new(),
    };
    for mut file in files {
        if options.hash_file_contents {
            let bytes =
                fs::read(&file.absolute).map_err(|source| Error::io(&file.absolute, source))?;
            if options.detect_binary_files && looks_binary(&bytes) {
                inspected.skipped.push(binary_skip(file.relative));
                continue;
            }
            file.content_hash = Some(hash_bytes(&bytes));
        } else if options.detect_binary_files && file_looks_binary(&file.absolute)? {
            inspected.skipped.push(binary_skip(file.relative));
            continue;
        }
        inspected.files.push(file);
    }
    Ok(inspected)
}

fn binary_skip(relative: String) -> SkippedEntry {
    SkippedEntry {
        relative,
        kind: SkipKind::Binary,
        detail: None,
    }
}

fn hash_bytes(bytes: &[u8]) -> String {
    let mut hasher = RevisionHasher::new();
    hasher.write(bytes);
    hasher.finish()
}

fn file_looks_binary(path: &Path) -> Result<bool> {
    let mut file = fs::File::open(path).map_err(|source| Error::io(path, source))?;
    let mut buffer = [0; 8192];
    let read = file
        .read(&mut buffer)
        .map_err(|source| Error::io(path, source))?;
    Ok(looks_binary(&buffer[..read]))
}
