use crate::p2_common::has_extension;
use crate::support::{BenchmarkCase, Fixture, RAW_FILES, measure_group, print_measurement};
use ignore::{WalkBuilder as IgnoreWalkBuilder, WalkState};
use jwalk::{WalkDir as JWalkDir, WalkDirGeneric as JWalkDirGeneric};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use weavatrix_scan::{
    ParallelMultiWalker, ParallelWalker, RootSymlinkPolicy, StatefulWalkBuilder, StatefulWalkEntry,
    WalkBuilder, WalkControl, WalkEntry, WalkEvent, WalkOptions, Walker,
};

pub(crate) fn benchmark_parallel_pull(fixture: &Fixture) {
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

pub(crate) fn benchmark_parallel_multi_stream(fixture: &Fixture) {
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

pub(crate) fn benchmark_directory_callbacks(fixture: &Fixture) {
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

pub(crate) fn benchmark_root_policy(fixture: &Fixture) {
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
