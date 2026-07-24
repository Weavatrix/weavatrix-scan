mod support;

use ignore::WalkBuilder;
use jwalk::WalkDir as JWalkDir;
use std::path::{Path, PathBuf};
use support::{
    BenchmarkCase, EXTENSIONS, Fixture, IGNORE_AWARE_FILES, RAW_FILES, SOURCE_FILES, measure,
    measure_group, print_measurement,
};
use walkdir::WalkDir;
use weavatrix_scan::{
    ParallelWalker, ScanOptions, Scanner, StandardSkips, WalkEntry, WalkOptions, Walker,
};

fn main() {
    let fixture = Fixture::new();

    println!("corpus=synthetic source_files={SOURCE_FILES} statistic=median runs=11 warmups=2");
    benchmark_raw_discovery(&fixture);
    benchmark_ignore_aware_discovery(&fixture);
    benchmark_rich_manifest(&fixture);
}

fn benchmark_raw_discovery(fixture: &Fixture) {
    let expected = walker_paths(&fixture.root);
    let mut cases = vec![
        BenchmarkCase::new("weavatrix-walker", || {
            checked_path_len(&walker_paths(&fixture.root), &expected)
        }),
        BenchmarkCase::new("weavatrix-parallel", || {
            checked_path_len(&parallel_paths(&fixture.root), &expected)
        }),
        BenchmarkCase::new("ignore", || {
            checked_path_len(&ignore_paths(&fixture.root), &expected)
        }),
        BenchmarkCase::new("walkdir", || {
            checked_path_len(&walkdir_paths(&fixture.root), &expected)
        }),
        BenchmarkCase::new("jwalk", || {
            checked_path_len(&jwalk_paths(&fixture.root), &expected)
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
type Paths = Vec<PathBuf>;

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

fn walker_paths(root: &Path) -> Paths {
    let mut paths = Walker::with_options(root, WalkOptions::default())
        .unwrap()
        .filter_map(Result::ok)
        .filter(WalkEntry::is_file)
        .filter(|entry| has_extension(entry.path()))
        .map(|entry| entry.relative_path().to_path_buf())
        .collect::<Vec<_>>();
    paths.sort_unstable();
    paths
}

fn parallel_paths(root: &Path) -> Paths {
    let mut paths = ParallelWalker::new(root)
        .walk()
        .unwrap()
        .entries
        .into_iter()
        .filter(WalkEntry::is_file)
        .filter(|entry| has_extension(entry.path()))
        .map(|entry| entry.relative_path().to_path_buf())
        .collect::<Vec<_>>();
    paths.sort_unstable();
    paths
}

fn ignore_paths(root: &Path) -> Paths {
    let mut builder = WalkBuilder::new(root);
    builder.standard_filters(false);
    relative_paths(
        root,
        builder
            .build()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_type().is_some_and(|kind| kind.is_file()))
            .filter(|entry| has_extension(entry.path()))
            .map(ignore::DirEntry::into_path),
    )
}

fn walkdir_paths(root: &Path) -> Paths {
    relative_paths(
        root,
        WalkDir::new(root)
            .follow_links(false)
            .into_iter()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_type().is_file())
            .filter(|entry| has_extension(entry.path()))
            .map(walkdir::DirEntry::into_path),
    )
}

fn jwalk_paths(root: &Path) -> Paths {
    relative_paths(
        root,
        JWalkDir::new(root)
            .sort(false)
            .skip_hidden(false)
            .into_iter()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_type().is_file())
            .filter(|entry| has_extension(&entry.path()))
            .map(|entry| entry.path()),
    )
}

fn relative_paths(root: &Path, paths: impl Iterator<Item = PathBuf>) -> Paths {
    let mut paths = paths
        .map(|path| path.strip_prefix(root).unwrap().to_path_buf())
        .collect::<Vec<_>>();
    paths.sort_unstable();
    paths
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
