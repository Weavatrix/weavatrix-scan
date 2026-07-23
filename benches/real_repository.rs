mod support;

use ignore::WalkBuilder;
use std::path::{Path, PathBuf};
use support::{BenchmarkCase, measure_group, print_measurement};
use weavatrix_scan::{ScanOptions, Scanner, StandardSkips};

const EXTENSIONS: &[&str] = &[
    "c", "cc", "cpp", "go", "h", "hpp", "java", "js", "json", "md", "py", "rs", "sh", "sql",
    "toml", "ts", "tsx", "yaml", "yml",
];

fn main() {
    let root = benchmark_root();
    println!(
        "corpus=real root={} statistic=median runs=11 warmups=2",
        root.display()
    );
    let ours = weavatrix_paths(&root);
    let competitor = ignore_paths(&root);
    if ours != competitor {
        let only_ours = ours
            .iter()
            .filter(|path| competitor.binary_search(path).is_err())
            .collect::<Vec<_>>();
        let only_competitor = competitor
            .iter()
            .filter(|path| ours.binary_search(path).is_err())
            .collect::<Vec<_>>();
        eprintln!("only_weavatrix={only_ours:?}");
        eprintln!("only_ignore={only_competitor:?}");
    }
    assert_eq!(ours, competitor);

    let mut cases = vec![
        BenchmarkCase::new("weavatrix-scan", || {
            let mut options = ScanOptions::default()
                .with_extensions(EXTENSIONS)
                .metadata_only();
            options.standard_skips = StandardSkips::Disabled;
            options.max_file_bytes = u64::MAX;
            Scanner::new(&root)
                .options(options)
                .scan()
                .unwrap()
                .files
                .len()
        }),
        BenchmarkCase::new("ignore", || ignore_count(&root)),
    ];
    let results = measure_group(&mut cases);
    assert_eq!(results[0].count, results[1].count);
    for (case, result) in cases.iter().zip(results) {
        print_measurement("ignore-aware-real", case.name, &result);
    }
}

fn weavatrix_paths(root: &Path) -> Vec<String> {
    let mut options = ScanOptions::default()
        .with_extensions(EXTENSIONS)
        .metadata_only();
    options.standard_skips = StandardSkips::Disabled;
    options.max_file_bytes = u64::MAX;
    Scanner::new(root)
        .options(options)
        .scan()
        .unwrap()
        .files
        .into_iter()
        .map(|file| file.relative)
        .collect()
}

fn ignore_paths(root: &Path) -> Vec<String> {
    let mut builder = WalkBuilder::new(root);
    builder
        .add_custom_ignore_filename(".weavatrixignore")
        .git_global(false)
        .git_exclude(false)
        .hidden(false)
        .parents(false)
        .require_git(false);
    let mut paths = builder
        .build()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_some_and(|kind| kind.is_file()))
        .filter(|entry| has_extension(entry.path()))
        .map(|entry| {
            entry
                .path()
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/")
        })
        .collect::<Vec<_>>();
    paths.sort_unstable();
    paths
}

fn benchmark_root() -> PathBuf {
    std::env::var_os("WEAVATRIX_BENCH_ROOT")
        .map_or_else(|| std::env::current_dir().unwrap(), PathBuf::from)
}

fn ignore_count(root: &Path) -> usize {
    let mut builder = WalkBuilder::new(root);
    builder
        .add_custom_ignore_filename(".weavatrixignore")
        .git_global(false)
        .git_exclude(false)
        .hidden(false)
        .parents(false)
        .require_git(false);
    builder
        .build()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_some_and(|kind| kind.is_file()))
        .filter(|entry| has_extension(entry.path()))
        .count()
}

fn has_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|value| value.to_str())
        .is_some_and(|value| EXTENSIONS.contains(&value.to_ascii_lowercase().as_str()))
}
