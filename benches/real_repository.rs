mod support;

use ignore::WalkBuilder;
use std::path::{Path, PathBuf};
use support::{
    BenchmarkCase, benchmark_runs, benchmark_warmups, dirwalk_paths, ignore_paths, jwalk_paths,
    measure_group, parallel_paths, print_measurement, std_read_dir_paths, walkdir_paths,
    walker_paths,
};
use walkdir::WalkDir;
use weavatrix_scan::{ScanOptions, Scanner, StandardSkips};

const EXTENSIONS: &[&str] = &[
    "c", "cc", "cpp", "go", "h", "hpp", "java", "js", "json", "md", "py", "rs", "sh", "sql",
    "toml", "ts", "tsx", "yaml", "yml",
];

fn main() {
    let root = benchmark_root();
    println!(
        "corpus=real statistic=median runs={} warmups={}",
        benchmark_runs(),
        benchmark_warmups()
    );
    match std::env::var("WEAVATRIX_BENCH_MODE").as_deref() {
        Ok("raw") => benchmark_raw(&root),
        Ok("manifest") => benchmark_manifest(&root),
        Ok("no-ignore") => benchmark_no_ignore(&root),
        _ => {
            benchmark_raw(&root);
            benchmark_manifest(&root);
            benchmark_no_ignore(&root);
        }
    }
}

fn benchmark_raw(root: &Path) {
    let expected = walker_paths(root, EXTENSIONS);
    let mut cases = vec![
        BenchmarkCase::new("weavatrix-walker", || walker_paths(root, EXTENSIONS).len()),
        BenchmarkCase::new("weavatrix-parallel", || {
            parallel_paths(root, EXTENSIONS).len()
        }),
        BenchmarkCase::new("ignore", || ignore_paths(root, EXTENSIONS).len()),
        BenchmarkCase::new("walkdir", || walkdir_paths(root, EXTENSIONS).len()),
        BenchmarkCase::new("jwalk", || jwalk_paths(root, EXTENSIONS).len()),
        BenchmarkCase::new("dirwalk", || dirwalk_paths(root, EXTENSIONS).len()),
        BenchmarkCase::new("std-read-dir", || {
            std_read_dir_paths(root, EXTENSIONS).len()
        }),
    ];
    assert_eq!(parallel_paths(root, EXTENSIONS), expected);
    assert_eq!(ignore_paths(root, EXTENSIONS), expected);
    assert_eq!(walkdir_paths(root, EXTENSIONS), expected);
    assert_eq!(jwalk_paths(root, EXTENSIONS), expected);
    assert_eq!(dirwalk_paths(root, EXTENSIONS), expected);
    assert_eq!(std_read_dir_paths(root, EXTENSIONS), expected);
    let results = measure_group(&mut cases);
    assert!(results.iter().all(|result| result.count == expected.len()));
    for (case, result) in cases.iter().zip(results) {
        print_measurement("raw-real", case.name, &result);
    }
}

fn benchmark_manifest(root: &Path) {
    let ours = weavatrix_compact_manifest(root, 0);
    let competitor = ignore_manifest(root);
    if ours != competitor {
        let only_ours = ours
            .iter()
            .filter(|path| competitor.binary_search(path).is_err())
            .count();
        let only_competitor = competitor
            .iter()
            .filter(|path| ours.binary_search(path).is_err())
            .count();
        eprintln!("manifest mismatch: only_weavatrix={only_ours} only_ignore={only_competitor}");
        panic!("real-repository manifests differ");
    }
    let dirwalk = dirwalk_manifest(root);
    let dirwalk_matches = dirwalk == competitor;
    if !dirwalk_matches {
        let only_dirwalk = dirwalk
            .iter()
            .filter(|path| competitor.binary_search(path).is_err())
            .count();
        let only_oracle = competitor
            .iter()
            .filter(|path| dirwalk.binary_search(path).is_err())
            .count();
        eprintln!(
            "dirwalk manifest excluded from timing: only_dirwalk={only_dirwalk} only_oracle={only_oracle}"
        );
    }

    let mut cases = if std::env::var_os("WEAVATRIX_BENCH_VARIANT").as_deref()
        == Some(std::ffi::OsStr::new("full"))
    {
        vec![
            BenchmarkCase::new("weavatrix-full-parallel", || {
                weavatrix_full_manifest(root, 0).len()
            }),
            BenchmarkCase::new("ignore", || ignore_manifest(root).len()),
        ]
    } else {
        vec![
            BenchmarkCase::new("weavatrix-compact-parallel", || {
                weavatrix_compact_manifest(root, 0).len()
            }),
            BenchmarkCase::new("weavatrix-compact-serial", || {
                weavatrix_compact_manifest(root, 1).len()
            }),
            BenchmarkCase::new("weavatrix-full-parallel", || {
                weavatrix_full_manifest(root, 0).len()
            }),
            BenchmarkCase::new("weavatrix-full-serial", || {
                weavatrix_full_manifest(root, 1).len()
            }),
            BenchmarkCase::new("ignore", || ignore_manifest(root).len()),
        ]
    };
    if dirwalk_matches {
        cases.push(BenchmarkCase::new("dirwalk", || {
            dirwalk_manifest(root).len()
        }));
    }
    let results = measure_group(&mut cases);
    assert!(results.iter().all(|result| result.count == ours.len()));
    for (case, result) in cases.iter().zip(results) {
        print_measurement("ignore-aware-real", case.name, &result);
    }
}

fn benchmark_no_ignore(root: &Path) {
    let expected = walkdir_manifest(root);
    assert_eq!(weavatrix_no_ignore_manifest(root), expected);
    let mut cases = vec![
        BenchmarkCase::new("weavatrix-compact", || {
            weavatrix_no_ignore_manifest(root).len()
        }),
        BenchmarkCase::new("walkdir", || walkdir_manifest(root).len()),
    ];
    let results = measure_group(&mut cases);
    assert!(results.iter().all(|result| result.count == expected.len()));
    for (case, result) in cases.iter().zip(results) {
        print_measurement("no-ignore-real", case.name, &result);
    }
}

type Manifest = Vec<(String, u64)>;

fn scan_options(parallelism: usize) -> ScanOptions {
    let mut options = ScanOptions::default()
        .with_extensions(EXTENSIONS)
        .with_parallelism(parallelism)
        .metadata_only()
        .selected_files_only();
    options.standard_skips = StandardSkips::Disabled;
    options.max_file_bytes = u64::MAX;
    options
}

fn weavatrix_compact_manifest(root: &Path, parallelism: usize) -> Manifest {
    Scanner::new(root)
        .options(scan_options(parallelism))
        .scan_compact()
        .unwrap()
        .files
        .into_iter()
        .map(|file| (file.relative.into(), file.bytes))
        .collect()
}

fn weavatrix_full_manifest(root: &Path, parallelism: usize) -> Manifest {
    Scanner::new(root)
        .options(scan_options(parallelism))
        .scan()
        .unwrap()
        .files
        .into_iter()
        .map(|file| (file.relative, file.bytes))
        .collect()
}

fn weavatrix_no_ignore_manifest(root: &Path) -> Manifest {
    Scanner::new(root)
        .options(scan_options(0).with_ignore_files(std::iter::empty::<&str>()))
        .scan_compact()
        .unwrap()
        .files
        .into_iter()
        .map(|file| (file.relative.into(), file.bytes))
        .collect()
}

fn ignore_manifest(root: &Path) -> Manifest {
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
            let bytes = entry.metadata().unwrap().len();
            let relative = entry
                .path()
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            (relative, bytes)
        })
        .collect::<Vec<_>>();
    paths.sort_unstable();
    paths
}

fn walkdir_manifest(root: &Path) -> Manifest {
    let mut paths = WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .filter(|entry| has_extension(entry.path()))
        .map(|entry| {
            let bytes = entry.metadata().unwrap().len();
            let relative = entry
                .path()
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            (relative, bytes)
        })
        .collect::<Vec<_>>();
    paths.sort_unstable();
    paths
}

fn dirwalk_manifest(root: &Path) -> Manifest {
    let result = dirwalk::WalkBuilder::new(root)
        .hidden(true)
        .extensions(EXTENSIONS)
        .gitignore(true)
        .build()
        .unwrap();
    let mut paths = result
        .entries
        .into_iter()
        .filter(|entry| !entry.is_dir)
        .map(|entry| (entry.relative_path.replace('\\', "/"), entry.size))
        .collect::<Vec<_>>();
    paths.sort_unstable();
    paths
}

fn benchmark_root() -> PathBuf {
    std::env::var_os("WEAVATRIX_BENCH_ROOT")
        .map_or_else(|| std::env::current_dir().unwrap(), PathBuf::from)
}

fn has_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|value| value.to_str())
        .is_some_and(|value| EXTENSIONS.contains(&value.to_ascii_lowercase().as_str()))
}
