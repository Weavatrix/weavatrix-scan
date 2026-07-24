mod support;

use ignore::{WalkBuilder as IgnoreWalkBuilder, WalkState};
use jwalk::{WalkDir as JWalkDir, WalkDirGeneric as JWalkDirGeneric};
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use support::{
    BenchmarkCase, EXTENSIONS, Fixture, IGNORE_AWARE_FILES, RAW_FILES, SOURCE_FILES, measure_group,
    print_measurement,
};
use weavatrix_scan::{
    ChangedContentVisitOutcome, ContentFileStatus, ContentValidationPolicy, ContentVisitControl,
    ContentVisitEvent, MultiScanner, ParallelMultiWalker, ParallelWalker, RootSymlinkPolicy,
    ScanOptions, Scanner, StatefulWalkBuilder, StatefulWalkEntry, WalkBuilder, WalkControl,
    WalkEntry, WalkEvent, WalkOptions, Walker, WatchEvent, WatchEventKind, WatcherEventAdapter,
};

fn main() {
    let fixture = Fixture::new();
    let second_fixture = Fixture::new();

    println!(
        "corpus=synthetic source_files={SOURCE_FILES} statistic=median runs=11 warmups=2 suite=p2"
    );
    benchmark_parallel_pull(&fixture);
    benchmark_directory_callbacks(&fixture);
    benchmark_parallel_multi_stream(&fixture);
    benchmark_content_visit(&fixture);
    benchmark_multi_content_visit(&fixture, &second_fixture);
    benchmark_root_policy(&fixture);
    benchmark_watcher_adapter(&fixture);
}

fn benchmark_multi_content_visit(first: &Fixture, second: &Fixture) {
    let mut cases = vec![
        BenchmarkCase::new("weavatrix-multi-content-revision", || {
            weavatrix_multi_content_count(&first.root, &second.root, false)
        }),
        BenchmarkCase::new("weavatrix-multi-content-streaming", || {
            weavatrix_multi_content_count(&first.root, &second.root, true)
        }),
        BenchmarkCase::new("ignore-multi-content-verified", || {
            ignore_multi_content_count(&first.root, &second.root, SnapshotChecks::BeforeAndAfter)
        }),
    ];
    let results = measure_group(&mut cases);
    for (case, result) in cases.iter().zip(results) {
        assert_eq!(result.count, IGNORE_AWARE_FILES * 2);
        print_measurement("parallel-multi-content-visit", case.name, &result);
    }
}

fn benchmark_content_visit(fixture: &Fixture) {
    let mut cases = vec![
        BenchmarkCase::new("weavatrix-content-fast", || {
            weavatrix_content_count(&fixture.root, ContentValidationPolicy::Fast, 0, false)
        }),
        BenchmarkCase::new("weavatrix-content-strict", || {
            weavatrix_content_count(&fixture.root, ContentValidationPolicy::Strict, 0, false)
        }),
        BenchmarkCase::new("weavatrix-content-streaming-fast", || {
            weavatrix_content_count(&fixture.root, ContentValidationPolicy::Fast, 0, true)
        }),
        BenchmarkCase::new("weavatrix-content-streaming-strict", || {
            weavatrix_content_count(&fixture.root, ContentValidationPolicy::Strict, 0, true)
        }),
        BenchmarkCase::new("ignore-content-visit", || {
            ignore_content_count(&fixture.root, SnapshotChecks::None)
        }),
        BenchmarkCase::new("ignore-content-open-verified", || {
            ignore_content_count(&fixture.root, SnapshotChecks::Before)
        }),
        BenchmarkCase::new("ignore-content-verified", || {
            ignore_content_count(&fixture.root, SnapshotChecks::BeforeAndAfter)
        }),
    ];
    let results = measure_group(&mut cases);
    for (case, result) in cases.iter().zip(results) {
        assert_eq!(result.count, IGNORE_AWARE_FILES);
        print_measurement("parallel-content-visit", case.name, &result);
    }
}

fn weavatrix_content_count(
    root: &Path,
    validation: ContentValidationPolicy,
    workers: usize,
    streaming: bool,
) -> usize {
    let files = Arc::new(AtomicUsize::new(0));
    let bytes = Arc::new(AtomicU64::new(0));
    let scanner = Scanner::new(root).options(
        ScanOptions::default()
            .with_extensions(EXTENSIONS.iter().copied())
            .selected_files_only()
            .metadata_only()
            .with_content_parallelism(workers)
            .with_content_validation(validation),
    );
    let report = if streaming {
        scanner.visit_content_streaming({
            let files = Arc::clone(&files);
            let bytes = Arc::clone(&bytes);
            move |_| {
                let files = Arc::clone(&files);
                let bytes = Arc::clone(&bytes);
                move |event| count_content_event(&event, &files, &bytes)
            }
        })
    } else {
        scanner.visit_content({
            let files = Arc::clone(&files);
            let bytes = Arc::clone(&bytes);
            move |_| {
                let files = Arc::clone(&files);
                let bytes = Arc::clone(&bytes);
                move |event| count_content_event(&event, &files, &bytes)
            }
        })
    }
    .unwrap();
    assert_eq!(
        usize::try_from(report.completed).unwrap(),
        files.load(Ordering::Relaxed)
    );
    std::hint::black_box(bytes.load(Ordering::Relaxed));
    files.load(Ordering::Relaxed)
}

fn count_content_event(
    event: &ContentVisitEvent<'_>,
    files: &AtomicUsize,
    bytes: &AtomicU64,
) -> ContentVisitControl {
    match event {
        ContentVisitEvent::Chunk { bytes: chunk, .. } => {
            bytes.fetch_add(u64::try_from(chunk.len()).unwrap(), Ordering::Relaxed);
        }
        ContentVisitEvent::FileEnd {
            status: ContentFileStatus::Selected,
            ..
        } => {
            files.fetch_add(1, Ordering::Relaxed);
        }
        ContentVisitEvent::FileStart { .. } | ContentVisitEvent::FileEnd { .. } => {}
    }
    ContentVisitControl::Continue
}

fn weavatrix_multi_content_count(first: &Path, second: &Path, streaming: bool) -> usize {
    let files = Arc::new(AtomicUsize::new(0));
    let bytes = Arc::new(AtomicU64::new(0));
    let scanner = MultiScanner::new(first)
        .add_root(second)
        .with_root_parallelism(2)
        .options(
            ScanOptions::default()
                .with_extensions(EXTENSIONS.iter().copied())
                .selected_files_only()
                .metadata_only()
                .with_content_validation(ContentValidationPolicy::Strict),
        );
    let report = if streaming {
        scanner.visit_content_streaming({
            let files = Arc::clone(&files);
            let bytes = Arc::clone(&bytes);
            move |_, _| {
                let files = Arc::clone(&files);
                let bytes = Arc::clone(&bytes);
                move |event| count_content_event(&event, &files, &bytes)
            }
        })
    } else {
        scanner.visit_content({
            let files = Arc::clone(&files);
            let bytes = Arc::clone(&bytes);
            move |_, _| {
                let files = Arc::clone(&files);
                let bytes = Arc::clone(&bytes);
                move |event| count_content_event(&event, &files, &bytes)
            }
        })
    }
    .unwrap();
    assert_eq!(report.len(), 2);
    std::hint::black_box(bytes.load(Ordering::Relaxed));
    files.load(Ordering::Relaxed)
}

#[derive(Clone, Copy)]
enum SnapshotChecks {
    None,
    Before,
    BeforeAndAfter,
}

fn ignore_content_count(root: &Path, snapshot_checks: SnapshotChecks) -> usize {
    let mut builder = IgnoreWalkBuilder::new(root);
    builder
        .add_custom_ignore_filename(".weavatrixignore")
        .require_git(false);
    run_ignore_content(&builder, snapshot_checks)
}

fn ignore_multi_content_count(
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

fn benchmark_parallel_pull(fixture: &Fixture) {
    let expected = jwalk_paths(&fixture.root);
    let mut cases = vec![
        BenchmarkCase::new("weavatrix-pull-1", || {
            checked_path_len(&parallel_pull_paths(&fixture.root, 1), &expected)
        }),
        BenchmarkCase::new("weavatrix-pull-64", || {
            checked_path_len(&parallel_pull_paths(&fixture.root, 64), &expected)
        }),
        BenchmarkCase::new("weavatrix-pull-1024", || {
            checked_path_len(&parallel_pull_paths(&fixture.root, 1024), &expected)
        }),
        BenchmarkCase::new("weavatrix-ordered-1024", || {
            checked_path_len(&parallel_ordered_paths(&fixture.root, 1024), &expected)
        }),
        BenchmarkCase::new("jwalk", || {
            checked_path_len(&jwalk_paths(&fixture.root), &expected)
        }),
    ];
    let results = measure_group(&mut cases);
    for (case, result) in cases.iter().zip(results) {
        assert_eq!(result.count, RAW_FILES);
        print_measurement("parallel-pull", case.name, &result);
    }
}

fn benchmark_parallel_multi_stream(fixture: &Fixture) {
    let expected = RAW_FILES * 2;
    let mut cases = vec![
        BenchmarkCase::new("weavatrix-multi-stream", || {
            weavatrix_multi_stream_count(&fixture.root)
        }),
        BenchmarkCase::new("ignore-multi-stream", || {
            ignore_multi_stream_count(&fixture.root)
        }),
    ];
    let results = measure_group(&mut cases);
    for (case, result) in cases.iter().zip(results) {
        assert_eq!(result.count, expected);
        print_measurement("parallel-multi-stream", case.name, &result);
    }
}

fn weavatrix_multi_stream_count(root: &Path) -> usize {
    let count = Arc::new(AtomicUsize::new(0));
    let callback_count = Arc::clone(&count);
    ParallelMultiWalker::new(root)
        .add_root(root)
        .with_root_parallelism(2)
        .visit(move |event| {
            if let WalkEvent::Entry(entry) = event.event
                && entry.is_file()
                && has_extension(entry.path())
            {
                callback_count.fetch_add(1, Ordering::Relaxed);
            }
            WalkControl::Continue
        })
        .unwrap();
    count.load(Ordering::Relaxed)
}

fn ignore_multi_stream_count(root: &Path) -> usize {
    let count = Arc::new(AtomicUsize::new(0));
    let mut builder = IgnoreWalkBuilder::new(root);
    builder
        .add(root)
        .hidden(false)
        .ignore(false)
        .git_global(false)
        .git_ignore(false)
        .git_exclude(false)
        .parents(false)
        .require_git(false);
    builder.build_parallel().run(|| {
        let callback_count = Arc::clone(&count);
        Box::new(move |result| {
            if let Ok(entry) = result
                && entry.file_type().is_some_and(|kind| kind.is_file())
                && has_extension(entry.path())
            {
                callback_count.fetch_add(1, Ordering::Relaxed);
            }
            WalkState::Continue
        })
    });
    count.load(Ordering::Relaxed)
}

fn benchmark_directory_callbacks(fixture: &Fixture) {
    let mut cases = vec![
        BenchmarkCase::new("weavatrix-builder", || {
            builder_file_count(&fixture.root, CallbackMode::None)
        }),
        BenchmarkCase::new("weavatrix-stateless", || {
            builder_file_count(&fixture.root, CallbackMode::Stateless)
        }),
        BenchmarkCase::new("weavatrix-stateful", || {
            builder_file_count(&fixture.root, CallbackMode::Stateful)
        }),
        BenchmarkCase::new("weavatrix-stateful-batch", || {
            stateful_batch_file_count(&fixture.root)
        }),
        BenchmarkCase::new("weavatrix-stateful-batch-parallel", || {
            parallel_stateful_batch_file_count(&fixture.root)
        }),
        BenchmarkCase::new("jwalk-stateful-batch", || {
            jwalk_stateful_batch_file_count(&fixture.root)
        }),
    ];
    let results = measure_group(&mut cases);
    for (case, result) in cases.iter().zip(results) {
        assert_eq!(result.count, RAW_FILES);
        print_measurement("directory-callback", case.name, &result);
    }
}

fn benchmark_root_policy(fixture: &Fixture) {
    let mut cases = vec![
        BenchmarkCase::new("weavatrix-root-follow", || {
            walker_file_count(&fixture.root, RootSymlinkPolicy::Follow)
        }),
        BenchmarkCase::new("weavatrix-root-reject", || {
            walker_file_count(&fixture.root, RootSymlinkPolicy::Reject)
        }),
    ];
    let results = measure_group(&mut cases);
    for (case, result) in cases.iter().zip(results) {
        assert_eq!(result.count, RAW_FILES);
        print_measurement("root-policy", case.name, &result);
    }
}

fn benchmark_watcher_adapter(fixture: &Fixture) {
    const EVENT_COUNT: usize = 1_024;

    let options = ScanOptions::default().with_extensions(EXTENSIONS);
    let report = Scanner::new(&fixture.root)
        .options(options.clone())
        .scan()
        .unwrap();
    let cache = report.to_cache();
    assert_eq!(cache.entries.len(), SOURCE_FILES);
    let adapter = WatcherEventAdapter::with_options(&fixture.root, &options).unwrap();
    let events = watcher_events(&fixture.root, EVENT_COUNT);
    let full_rescan = vec![WatchEvent::new("", WatchEventKind::Rescan)];
    let incremental_plan = adapter.plan(events.clone());
    let changed_plan = adapter.plan(
        events
            .iter()
            .map(|event| WatchEvent::new(event.path.clone(), WatchEventKind::Modify)),
    );
    assert!(!incremental_plan.full_rescan);
    assert_eq!(
        incremental_plan.changed.len() + incremental_plan.removed.len(),
        EVENT_COUNT
    );
    let mut cases = vec![
        BenchmarkCase::new("weavatrix-plan", || {
            let plan = adapter.plan(events.clone());
            assert!(!plan.full_rescan);
            plan.changed.len() + plan.removed.len()
        }),
        BenchmarkCase::new("weavatrix-cache-apply", || {
            let mut next_cache = cache.clone();
            next_cache.apply_watch_plan(&incremental_plan)
        }),
        BenchmarkCase::new("weavatrix-incremental", || {
            let plan = adapter.plan(events.clone());
            let mut next_cache = cache.clone();
            next_cache.apply_watch_plan(&plan)
        }),
        BenchmarkCase::new("weavatrix-full-rescan", || {
            let plan = adapter.plan(full_rescan.clone());
            assert!(plan.full_rescan);
            let mut next_cache = cache.clone();
            next_cache.apply_watch_plan(&plan)
        }),
        BenchmarkCase::new("weavatrix-changed-scan", || {
            Scanner::new(&fixture.root)
                .options(options.clone())
                .scan_watch_plan(&report, &changed_plan)
                .unwrap()
                .files
                .len()
        }),
        BenchmarkCase::new("weavatrix-changed-content-streaming", || {
            let outcome = Scanner::new(&fixture.root)
                .options(options.clone())
                .visit_changed_content_streaming(&changed_plan, |_| {
                    |event| {
                        if let ContentVisitEvent::Chunk { bytes, .. } = event {
                            std::hint::black_box(bytes);
                        }
                        ContentVisitControl::Continue
                    }
                })
                .unwrap();
            let ChangedContentVisitOutcome::Visited(report) = outcome else {
                panic!("file-only benchmark plan unexpectedly required a full rescan");
            };
            usize::try_from(report.content.completed).unwrap()
        }),
        BenchmarkCase::new("weavatrix-full-scan", || {
            Scanner::new(&fixture.root)
                .options(options.clone())
                .scan()
                .unwrap()
                .files
                .len()
        }),
    ];
    let results = measure_group(&mut cases);
    for (case, result) in cases.iter().zip(results) {
        print_count("watcher-adapter", case.name, "items", &result);
    }
}

fn parallel_ordered_paths(root: &Path, capacity: usize) -> Vec<PathBuf> {
    let mut paths = ParallelWalker::new(root)
        .into_iter_ordered_bounded(capacity)
        .filter_map(Result::ok)
        .filter(WalkEntry::is_file)
        .filter(|entry| has_extension(entry.path()))
        .map(|entry| entry.relative_path().to_path_buf())
        .collect::<Vec<_>>();
    paths.sort_unstable();
    paths
}

fn parallel_pull_paths(root: &Path, capacity: usize) -> Vec<PathBuf> {
    let mut paths = ParallelWalker::new(root)
        .into_iter_bounded(capacity)
        .filter_map(Result::ok)
        .filter(WalkEntry::is_file)
        .filter(|entry| has_extension(entry.path()))
        .map(|entry| entry.relative_path().to_path_buf())
        .collect::<Vec<_>>();
    paths.sort_unstable();
    paths
}

fn jwalk_paths(root: &Path) -> Vec<PathBuf> {
    let mut paths = JWalkDir::new(root)
        .sort(false)
        .skip_hidden(false)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .filter(|entry| has_extension(&entry.path()))
        .map(|entry| entry.path().strip_prefix(root).unwrap().to_path_buf())
        .collect::<Vec<_>>();
    paths.sort_unstable();
    paths
}

fn checked_path_len(actual: &[PathBuf], expected: &[PathBuf]) -> usize {
    assert_eq!(actual, expected);
    actual.len()
}

#[derive(Clone, Copy)]
enum CallbackMode {
    None,
    Stateless,
    Stateful,
}

fn builder_file_count(root: &Path, mode: CallbackMode) -> usize {
    let builder = WalkBuilder::new(root);
    let walker = match mode {
        CallbackMode::None => builder.build(),
        CallbackMode::Stateless => builder.filter_directories(|_| true).build(),
        CallbackMode::Stateful => builder.filter_directories_stateful(|_| true).build(),
    };
    walker
        .filter_map(Result::ok)
        .filter(WalkEntry::is_file)
        .filter(|entry| has_extension(entry.path()))
        .count()
}

fn stateful_batch_file_count(root: &Path) -> usize {
    StatefulWalkBuilder::<usize, ()>::new(root, 0)
        .process_read_dir(|_, _, directories, _| *directories += 1)
        .build()
        .unwrap()
        .filter_map(Result::ok)
        .filter(StatefulWalkEntry::is_file)
        .filter(|entry| has_extension(entry.path()))
        .count()
}

fn parallel_stateful_batch_file_count(root: &Path) -> usize {
    StatefulWalkBuilder::<usize, ()>::new(root, 0)
        .with_parallelism(0)
        .process_read_dir(|_, _, directories, _| *directories += 1)
        .build_parallel_ordered(1_024)
        .unwrap()
        .filter_map(Result::ok)
        .filter(StatefulWalkEntry::is_file)
        .filter(|entry| has_extension(entry.path()))
        .count()
}

fn jwalk_stateful_batch_file_count(root: &Path) -> usize {
    JWalkDirGeneric::<(usize, ())>::new(root)
        .process_read_dir(|_, _, directories, _| *directories += 1)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .filter(|entry| has_extension(&entry.path()))
        .count()
}

fn walker_file_count(root: &Path, policy: RootSymlinkPolicy) -> usize {
    Walker::with_options(
        root,
        WalkOptions::default().with_root_symlink_policy(policy),
    )
    .unwrap()
    .filter_map(Result::ok)
    .filter(WalkEntry::is_file)
    .filter(|entry| has_extension(entry.path()))
    .count()
}

fn watcher_events(root: &Path, count: usize) -> Vec<WatchEvent> {
    (0..count)
        .map(|index| {
            let directory = index % support::DIRECTORIES;
            let file = (index / support::DIRECTORIES) % support::FILES_PER_LANGUAGE;
            let path = root.join(format!("module_{directory:03}/file_{file:03}.rs"));
            let kind = if index % 2 == 0 {
                WatchEventKind::Modify
            } else {
                WatchEventKind::Remove
            };
            WatchEvent::new(path, kind)
        })
        .collect()
}

fn has_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|value| value.to_str())
        .is_some_and(|value| EXTENSIONS.contains(&value.to_ascii_lowercase().as_str()))
}

fn print_count(mode: &str, library: &str, unit: &str, measurement: &support::Measurement) {
    println!(
        "mode={mode} library={library} {unit}={} median_ms={:.3} min_ms={:.3}",
        measurement.count,
        measurement.median.as_secs_f64() * 1_000.0,
        measurement.minimum.as_secs_f64() * 1_000.0
    );
}
