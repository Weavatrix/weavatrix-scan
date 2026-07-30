use crate::p2_common::has_extension;
use ignore::{WalkBuilder as IgnoreWalkBuilder, WalkState};
use std::io::Read as _;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

#[derive(Clone, Copy)]
pub(crate) enum SnapshotChecks {
    None,
    Before,
    BeforeAndAfter,
}

pub(crate) fn ignore_content_count(root: &Path, snapshot_checks: SnapshotChecks) -> usize {
    let mut builder = IgnoreWalkBuilder::new(root);
    builder
        .add_custom_ignore_filename(".weavatrixignore")
        .require_git(false);
    run_ignore_content(&builder, snapshot_checks)
}

pub(crate) fn ignore_multi_content_count(
    first: &Path,
    second: &Path,
    snapshot_checks: SnapshotChecks,
) -> usize {
    let mut builder = IgnoreWalkBuilder::new(first);
    builder
        .add(second)
        .add_custom_ignore_filename(".weavatrixignore")
        .require_git(false);
    run_ignore_content(&builder, snapshot_checks)
}

fn run_ignore_content(builder: &IgnoreWalkBuilder, snapshot_checks: SnapshotChecks) -> usize {
    let files = Arc::new(AtomicUsize::new(0));
    let bytes = Arc::new(AtomicU64::new(0));
    builder.build_parallel().run(|| {
        let files = Arc::clone(&files);
        let bytes = Arc::clone(&bytes);
        Box::new(move |result| {
            if let Ok(entry) = result
                && entry.file_type().is_some_and(|kind| kind.is_file())
                && has_extension(entry.path())
            {
                let mut file = std::fs::File::open(entry.path()).unwrap();
                let before = match snapshot_checks {
                    SnapshotChecks::None => None,
                    SnapshotChecks::Before | SnapshotChecks::BeforeAndAfter => {
                        Some(benchmark_snapshot(&file))
                    }
                };
                let mut buffer = vec![0_u8; 64 * 1024].into_boxed_slice();
                let mut read_bytes = 0_u64;
                loop {
                    let read = file.read(&mut buffer).unwrap();
                    if read == 0 {
                        break;
                    }
                    let read = u64::try_from(read).unwrap();
                    read_bytes = read_bytes.saturating_add(read);
                    bytes.fetch_add(read, Ordering::Relaxed);
                }
                if let Some(before) = before {
                    assert_eq!(read_bytes, before.bytes);
                    if matches!(snapshot_checks, SnapshotChecks::BeforeAndAfter) {
                        assert_eq!(benchmark_snapshot(&file), before);
                    }
                }
                files.fetch_add(1, Ordering::Relaxed);
            }
            WalkState::Continue
        })
    });
    std::hint::black_box(bytes.load(Ordering::Relaxed));
    files.load(Ordering::Relaxed)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct BenchmarkSnapshot {
    bytes: u64,
    modified: Option<u128>,
    file_system: Option<u64>,
    file: Option<u64>,
}

#[cfg(windows)]
fn benchmark_snapshot(file: &std::fs::File) -> BenchmarkSnapshot {
    let information = winapi_util::file::information(file).unwrap();
    BenchmarkSnapshot {
        bytes: information.file_size(),
        modified: information.last_write_time().map(u128::from),
        file_system: Some(information.volume_serial_number()),
        file: Some(information.file_index()),
    }
}

#[cfg(unix)]
fn benchmark_snapshot(file: &std::fs::File) -> BenchmarkSnapshot {
    use std::os::unix::fs::MetadataExt as _;

    let metadata = file.metadata().unwrap();
    BenchmarkSnapshot {
        bytes: metadata.len(),
        modified: u128::try_from(metadata.mtime())
            .ok()
            .zip(u128::try_from(metadata.mtime_nsec()).ok())
            .map(|(seconds, nanos)| seconds.saturating_mul(1_000_000_000) + nanos),
        file_system: Some(metadata.dev()),
        file: Some(metadata.ino()),
    }
}

#[cfg(not(any(unix, windows)))]
fn benchmark_snapshot(file: &std::fs::File) -> BenchmarkSnapshot {
    let metadata = file.metadata().unwrap();
    BenchmarkSnapshot {
        bytes: metadata.len(),
        modified: metadata
            .modified()
            .ok()
            .and_then(|modified| modified.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|duration| duration.as_nanos()),
        file_system: None,
        file: None,
    }
}
