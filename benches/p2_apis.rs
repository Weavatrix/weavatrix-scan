mod support;

use jwalk::WalkDir as JWalkDir;
use std::path::{Path, PathBuf};
use support::{
    BenchmarkCase, EXTENSIONS, Fixture, RAW_FILES, SOURCE_FILES, measure_group, print_measurement,
};
use weavatrix_scan::{
    ParallelWalker, RootSymlinkPolicy, ScanOptions, Scanner, WalkBuilder, WalkEntry, WalkOptions,
    Walker, WatchEvent, WatchEventKind, WatcherEventAdapter,
};

fn main() {
    let fixture = Fixture::new();

    println!(
        "corpus=synthetic source_files={SOURCE_FILES} statistic=median runs=11 warmups=2 suite=p2"
    );
    benchmark_parallel_pull(&fixture);
    benchmark_directory_callbacks(&fixture);
    benchmark_root_policy(&fixture);
    benchmark_watcher_adapter(&fixture);
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
    ];
    let results = measure_group(&mut cases);
    for (case, result) in cases.iter().zip(results) {
        print_count("watcher-adapter", case.name, "items", &result);
    }
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
