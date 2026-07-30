#[path = "support/scale_operations.rs"]
mod scale_operations;

use std::env;
use std::fs::File;
use std::hint::black_box;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;
use weavatrix_scan::{ScanOptions, Scanner};

use scale_operations::{
    content_visit, ignore, ignore_manifest, ignore_manifest_entries, jwalk, parallel_collected,
    parallel_stream, scanner, scanner_all, scanner_compact, scanner_manifest, scanner_rich,
    walkdir, walker,
};

const FILES_PER_DIRECTORY: usize = 500;
const DEFAULT_RUNS: usize = 5;

fn main() {
    let arguments = env::args_os().skip(1).collect::<Vec<_>>();
    if arguments.is_empty() {
        eprintln!("scale_large: skipped because no explicit corpus command was supplied");
        return;
    }
    let Some(command) = arguments.first().and_then(|value| value.to_str()) else {
        usage();
    };
    let Some(root) = arguments.get(1).map(PathBuf::from) else {
        usage();
    };
    match command {
        "prepare" => {
            let files = parse_usize(arguments.get(2), 300_000);
            prepare(&root, files);
        }
        "verify" => verify(&root),
        "walker" => benchmark(command, &root, arguments.get(2), || walker(&root)),
        "parallel-collected" => {
            benchmark(command, &root, arguments.get(2), || {
                parallel_collected(&root)
            });
        }
        "parallel-stream" => {
            benchmark(command, &root, arguments.get(2), || parallel_stream(&root));
        }
        "jwalk" => benchmark(command, &root, arguments.get(2), || jwalk(&root)),
        "walkdir" => benchmark(command, &root, arguments.get(2), || walkdir(&root)),
        "ignore" => benchmark(command, &root, arguments.get(2), || ignore(&root)),
        "ignore-manifest" => {
            benchmark(command, &root, arguments.get(2), || ignore_manifest(&root));
        }
        "scanner" => benchmark(command, &root, arguments.get(2), || scanner(&root)),
        "scanner-compact" => {
            benchmark(command, &root, arguments.get(2), || scanner_compact(&root));
        }
        "scanner-all" => benchmark(command, &root, arguments.get(2), || scanner_all(&root)),
        "scanner-rich" => benchmark(command, &root, arguments.get(2), || scanner_rich(&root)),
        "content-revision" => {
            benchmark(command, &root, arguments.get(2), || {
                content_visit(&root, false)
            });
        }
        "content-stream" => {
            benchmark(command, &root, arguments.get(2), || {
                content_visit(&root, true)
            });
        }
        _ => usage(),
    }
}

fn usage() -> ! {
    eprintln!(
        "usage: scale_large <prepare|verify|walker|parallel-collected|parallel-stream|jwalk|walkdir|ignore|ignore-manifest|scanner|scanner-compact|scanner-all|scanner-rich|content-revision|content-stream> <root> [files|runs]"
    );
    std::process::exit(2);
}

fn parse_usize(value: Option<&std::ffi::OsString>, default: usize) -> usize {
    value
        .and_then(|value| value.to_str())
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

fn prepare(root: &Path, file_count: usize) {
    let marker = root.join(".weavatrix-scale-count");
    if marker.exists() {
        let existing = std::fs::read_to_string(&marker)
            .ok()
            .and_then(|value| value.trim().parse::<usize>().ok());
        assert_eq!(
            existing,
            Some(file_count),
            "fixture exists with a different file count"
        );
        println!(
            "fixture=existing files={file_count} root={}",
            root.display()
        );
        return;
    }
    assert!(
        !root.exists(),
        "refusing to prepare into an existing unmarked directory"
    );
    std::fs::create_dir_all(root).unwrap();
    std::fs::write(root.join(".ignore"), "ignored/\n").unwrap();

    let directory_count = file_count.div_ceil(FILES_PER_DIRECTORY);
    let next = Arc::new(AtomicUsize::new(0));
    let root = Arc::new(root.to_path_buf());
    let workers = std::thread::available_parallelism()
        .map_or(1, std::num::NonZeroUsize::get)
        .min(16);
    std::thread::scope(|scope| {
        for _ in 0..workers {
            let next = Arc::clone(&next);
            let root = Arc::clone(&root);
            scope.spawn(move || {
                loop {
                    let directory = next.fetch_add(1, Ordering::Relaxed);
                    if directory >= directory_count {
                        break;
                    }
                    let parent = if directory.is_multiple_of(6) {
                        root.join("ignored")
                    } else {
                        root.join("source")
                    };
                    let parent = parent.join(format!("d{directory:04x}"));
                    std::fs::create_dir_all(&parent).unwrap();
                    let first = directory * FILES_PER_DIRECTORY;
                    let last = (first + FILES_PER_DIRECTORY).min(file_count);
                    for file in first..last {
                        File::create(parent.join(format!("f{file:06x}.rs"))).unwrap();
                    }
                }
            });
        }
    });
    std::fs::write(&marker, file_count.to_string()).unwrap();
    println!(
        "fixture=created files={file_count} directories={directory_count} root={}",
        root.display()
    );
}

fn benchmark(
    name: &str,
    root: &Path,
    runs: Option<&std::ffi::OsString>,
    mut operation: impl FnMut() -> usize,
) {
    let fixture_files = fixture_file_count(root);
    let runs = parse_usize(runs, DEFAULT_RUNS).max(1);
    let expected = operation();
    let mut samples = Vec::with_capacity(runs);
    for _ in 0..runs {
        let started = Instant::now();
        assert_eq!(operation(), expected);
        samples.push(started.elapsed().as_secs_f64() * 1_000.0);
    }
    samples.sort_by(f64::total_cmp);
    let median = samples[samples.len() / 2];
    black_box(expected);
    println!(
        "profile=scale-{fixture_files} library={name} files={expected} statistic=median runs={runs} elapsed_ms={median:.3}"
    );
}

fn fixture_file_count(root: &Path) -> usize {
    std::fs::read_to_string(root.join(".weavatrix-scale-count"))
        .unwrap()
        .trim()
        .parse()
        .unwrap()
}

fn verify(root: &Path) {
    let ours = scanner_manifest(
        root,
        ScanOptions::default()
            .with_extensions(["rs"])
            .metadata_only()
            .selected_files_only(),
    );
    let compact = Scanner::new(root)
        .options(
            ScanOptions::default()
                .with_extensions(["rs"])
                .metadata_only()
                .selected_files_only(),
        )
        .scan_compact()
        .unwrap()
        .files
        .into_iter()
        .map(|file| (file.relative.into(), file.bytes))
        .collect::<Vec<(String, u64)>>();
    let reference = ignore_manifest_entries(root);
    assert_eq!(ours, reference);
    assert_eq!(compact, reference);
    println!(
        "profile=scale-{} verification=exact-manifest files={} status=ok",
        fixture_file_count(root),
        ours.len()
    );
}
