mod support;

use ignore::WalkBuilder;
use std::path::{Path, PathBuf};
use support::{
    BenchmarkCase, EXTENSIONS, Fixture, IGNORE_AWARE_FILES, Paths, RAW_FILES, SOURCE_FILES,
    benchmark_runs, benchmark_warmups, dirwalk_paths, ignore_paths, jwalk_paths, measure,
    measure_group, parallel_paths, print_measurement, std_read_dir_paths, walkdir_paths,
    walker_paths,
};
use weavatrix_scan::{ScanOptions, Scanner, StandardSkips};

fn main() {
    let fixture = Fixture::new();

    println!(
        "corpus=synthetic source_files={SOURCE_FILES} statistic=median runs={} warmups={}",
        benchmark_runs(),
        benchmark_warmups()
    );
    benchmark_raw_discovery(&fixture);
    benchmark_ignore_aware_discovery(&fixture);
    benchmark_rich_manifest(&fixture);
}

fn benchmark_raw_discovery(fixture: &Fixture) {
    let expected = walker_paths(&fixture.root, EXTENSIONS);
    let mut cases = vec![
        BenchmarkCase::new("weavatrix-walker", || {
            checked_path_len(&walker_paths(&fixture.root, EXTENSIONS), &expected)
        }),
        BenchmarkCase::new("weavatrix-parallel", || {
            checked_path_len(&parallel_paths(&fixture.root, EXTENSIONS), &expected)
        }),
        BenchmarkCase::new("ignore", || {
            checked_path_len(&ignore_paths(&fixture.root, EXTENSIONS), &expected)
        }),
        BenchmarkCase::new("walkdir", || {
            checked_path_len(&walkdir_paths(&fixture.root, EXTENSIONS), &expected)
        }),
        BenchmarkCase::new("jwalk", || {
            checked_path_len(&jwalk_paths(&fixture.root, EXTENSIONS), &expected)
        }),
        BenchmarkCase::new("dirwalk", || {
            checked_path_len(&dirwalk_paths(&fixture.root, EXTENSIONS), &expected)
        }),
        BenchmarkCase::new("std-read-dir", || {
            checked_path_len(&std_read_dir_paths(&fixture.root, EXTENSIONS), &expected)
        }),
    ];
    let results = measure_group(&mut cases);
    for (case, result) in cases.iter().zip(results) {
        assert_eq!(result.count, RAW_FILES);
        print_measurement("raw-discovery", case.name, &result);
    }
}

fn benchmark_ignore_aware_discovery(fixture: &Fixture) {
    let expected = weavatrix_manifest(&fixture.root, true, 1);
    let mut cases = vec![
        BenchmarkCase::new("weavatrix-scan-serial", || {
            checked_len(&weavatrix_manifest(&fixture.root, true, 1), &expected)
        }),
        BenchmarkCase::new("weavatrix-scan-parallel", || {
            checked_len(&weavatrix_manifest(&fixture.root, true, 4), &expected)
        }),
        BenchmarkCase::new("ignore", || {
            checked_len(&ignore_manifest(&fixture.root, true), &expected)
        }),
    ];
    let results = measure_group(&mut cases);
    for (case, result) in cases.iter().zip(results) {
        assert_eq!(result.count, IGNORE_AWARE_FILES);
        print_measurement("ignore-aware", case.name, &result);
    }
}

fn benchmark_rich_manifest(fixture: &Fixture) {
    let ours = measure(|| {
        Scanner::new(&fixture.root)
            .options(ScanOptions::default().with_extensions(EXTENSIONS))
            .scan()
            .unwrap()
            .files
            .len()
    });
    assert_eq!(ours.count, SOURCE_FILES);
    print_measurement("rich-manifest", "weavatrix-scan", &ours);
}

type Manifest = Vec<(String, u64)>;

fn checked_len(actual: &Manifest, expected: &Manifest) -> usize {
    assert_eq!(actual, expected);
    actual.len()
}

fn weavatrix_manifest(root: &Path, respect_ignore_files: bool, parallelism: usize) -> Manifest {
    let mut options = ScanOptions::default()
        .with_extensions(EXTENSIONS)
        .with_parallelism(parallelism)
        .metadata_only();
    options = options.selected_files_only();
    options.standard_skips = StandardSkips::Disabled;
    if !respect_ignore_files {
        options = options.with_ignore_files(std::iter::empty::<&str>());
    }
    Scanner::new(root)
        .options(options)
        .scan()
        .unwrap()
        .files
        .into_iter()
        .map(|file| (file.relative, file.bytes))
        .collect()
}

fn checked_path_len(actual: &Paths, expected: &Paths) -> usize {
    assert_eq!(actual, expected);
    actual.len()
}

fn ignore_manifest(root: &Path, respect_ignore_files: bool) -> Manifest {
    let mut builder = WalkBuilder::new(root);
    if respect_ignore_files {
        builder
            .add_custom_ignore_filename(".weavatrixignore")
            .git_global(false)
            .git_exclude(false)
            .hidden(false)
            .parents(false)
            .require_git(false);
    } else {
        builder.standard_filters(false);
    }
    manifest(
        root,
        builder
            .build()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_type().is_some_and(|kind| kind.is_file()))
            .filter(|entry| has_extension(entry.path()))
            .map(|entry| {
                let bytes = entry.metadata().unwrap().len();
                (entry.path().to_owned(), bytes)
            }),
    )
}

fn manifest(root: &Path, files: impl Iterator<Item = (PathBuf, u64)>) -> Manifest {
    let mut files = files
        .map(|(path, bytes)| {
            let relative = path
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            (relative, bytes)
        })
        .collect::<Vec<_>>();
    files.sort_unstable_by(|left, right| left.0.cmp(&right.0));
    files
}

fn has_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|value| value.to_str())
        .is_some_and(|value| EXTENSIONS.contains(&value.to_ascii_lowercase().as_str()))
}
