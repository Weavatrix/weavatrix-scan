mod support;

use ignore::WalkBuilder;
use jwalk::WalkDir as JWalkDir;
use std::path::Path;
use support::{
    BenchmarkCase, EXTENSIONS, Fixture, IGNORE_AWARE_FILES, RAW_FILES, SOURCE_FILES, measure,
    measure_group, print_measurement,
};
use walkdir::WalkDir;
use weavatrix_scan::{ScanOptions, Scanner, StandardSkips};

fn main() {
    let fixture = Fixture::new();

    println!("corpus=synthetic source_files={SOURCE_FILES} statistic=median runs=11 warmups=2");
    benchmark_raw_discovery(&fixture);
    benchmark_ignore_aware_discovery(&fixture);
    benchmark_rich_manifest(&fixture);
}

fn benchmark_raw_discovery(fixture: &Fixture) {
    let mut cases = vec![
        BenchmarkCase::new("weavatrix-scan", || {
            let mut options = ScanOptions::default()
                .with_extensions(EXTENSIONS)
                .with_ignore_files(std::iter::empty::<&str>())
                .metadata_only();
            options.standard_skips = StandardSkips::Disabled;
            Scanner::new(&fixture.root)
                .options(options)
                .scan()
                .unwrap()
                .files
                .len()
        }),
        BenchmarkCase::new("ignore", || ignore_count(&fixture.root, false)),
        BenchmarkCase::new("walkdir", || walkdir_count(&fixture.root)),
        BenchmarkCase::new("jwalk", || jwalk_count(&fixture.root)),
    ];
    let results = measure_group(&mut cases);
    for (case, result) in cases.iter().zip(results) {
        assert_eq!(result.count, RAW_FILES);
        print_measurement("raw-discovery", case.name, &result);
    }
}

fn benchmark_ignore_aware_discovery(fixture: &Fixture) {
    let mut cases = vec![
        BenchmarkCase::new("weavatrix-scan", || {
            let mut options = ScanOptions::default()
                .with_extensions(EXTENSIONS)
                .metadata_only();
            options.standard_skips = StandardSkips::Disabled;
            Scanner::new(&fixture.root)
                .options(options)
                .scan()
                .unwrap()
                .files
                .len()
        }),
        BenchmarkCase::new("ignore", || ignore_count(&fixture.root, true)),
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

fn ignore_count(root: &Path, respect_ignore_files: bool) -> usize {
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
    builder
        .build()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_some_and(|kind| kind.is_file()))
        .filter(|entry| has_extension(entry.path()))
        .count()
}

fn walkdir_count(root: &Path) -> usize {
    WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .filter(|entry| has_extension(entry.path()))
        .count()
}

fn jwalk_count(root: &Path) -> usize {
    JWalkDir::new(root)
        .sort(true)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .filter(|entry| has_extension(&entry.path()))
        .count()
}

fn has_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|value| value.to_str())
        .is_some_and(|value| EXTENSIONS.contains(&value.to_ascii_lowercase().as_str()))
}
