use ignore::WalkBuilder;
use jwalk::WalkDir as JWalkDir;
use std::env;
use std::fs::File;
use std::hint::black_box;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;
use walkdir::WalkDir;
use weavatrix_scan::{
    ParallelWalker, ScanOptions, Scanner, StandardSkips, WalkControl, WalkEntry, WalkEvent, Walker,
};

const FILES_PER_DIRECTORY: usize = 500;
const DEFAULT_RUNS: usize = 5;

fn main() {
    let arguments = env::args_os().skip(1).collect::<Vec<_>>();
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
        _ => usage(),
    }
}

fn usage() -> ! {
    eprintln!(
        "usage: scale_large <prepare|verify|walker|parallel-collected|parallel-stream|jwalk|walkdir|ignore|ignore-manifest|scanner|scanner-compact|scanner-all|scanner-rich> <root> [files|runs]"
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

fn walker(root: &Path) -> usize {
    Walker::new(root)
        .unwrap()
        .filter_map(Result::ok)
        .filter(WalkEntry::is_file)
        .filter(|entry| is_source(entry.path()))
        .count()
}

fn parallel_collected(root: &Path) -> usize {
    ParallelWalker::new(root)
        .walk()
        .unwrap()
        .entries
        .into_iter()
        .filter(WalkEntry::is_file)
        .filter(|entry| is_source(entry.path()))
        .count()
}

fn parallel_stream(root: &Path) -> usize {
    let count = Arc::new(AtomicUsize::new(0));
    let visitor_count = Arc::clone(&count);
    ParallelWalker::new(root)
        .visit(move |event| {
            if let WalkEvent::Entry(entry) = event
                && entry.is_file()
                && is_source(entry.path())
            {
                visitor_count.fetch_add(1, Ordering::Relaxed);
            }
            WalkControl::Continue
        })
        .unwrap();
    count.load(Ordering::Relaxed)
}

fn jwalk(root: &Path) -> usize {
    JWalkDir::new(root)
        .sort(false)
        .skip_hidden(false)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .filter(|entry| is_source(&entry.path()))
        .count()
}

fn walkdir(root: &Path) -> usize {
    WalkDir::new(root)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .filter(|entry| is_source(entry.path()))
        .count()
}

fn ignore(root: &Path) -> usize {
    ignore_builder(root)
        .build()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_some_and(|kind| kind.is_file()))
        .filter(|entry| is_source(entry.path()))
        .count()
}

fn ignore_manifest(root: &Path) -> usize {
    ignore_manifest_entries(root).len()
}

fn ignore_manifest_entries(root: &Path) -> Vec<(String, u64)> {
    let mut files = ignore_builder(root)
        .build()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_some_and(|kind| kind.is_file()))
        .filter(|entry| is_source(entry.path()))
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
    files.sort_unstable();
    files
}

fn ignore_builder(root: &Path) -> WalkBuilder {
    let mut builder = WalkBuilder::new(root);
    builder
        .hidden(false)
        .git_global(false)
        .git_exclude(false)
        .parents(false)
        .require_git(false);
    builder
}

fn scanner(root: &Path) -> usize {
    scanner_with_options(
        root,
        ScanOptions::default()
            .with_extensions(["rs"])
            .metadata_only()
            .selected_files_only(),
    )
}

fn scanner_compact(root: &Path) -> usize {
    Scanner::new(root)
        .options(
            ScanOptions::default()
                .with_extensions(["rs"])
                .metadata_only()
                .selected_files_only(),
        )
        .scan_compact()
        .unwrap()
        .files
        .len()
}

fn scanner_all(root: &Path) -> usize {
    let mut options = ScanOptions::default()
        .with_extensions(["rs"])
        .with_ignore_files(std::iter::empty::<&str>())
        .metadata_only()
        .selected_files_only();
    options.standard_skips = StandardSkips::Disabled;
    scanner_with_options(root, options)
}

fn scanner_rich(root: &Path) -> usize {
    scanner_with_options(
        root,
        ScanOptions::default()
            .with_extensions(["rs"])
            .selected_files_only(),
    )
}

fn scanner_with_options(root: &Path, options: ScanOptions) -> usize {
    scanner_manifest(root, options).len()
}

fn scanner_manifest(root: &Path, options: ScanOptions) -> Vec<(String, u64)> {
    Scanner::new(root)
        .options(options)
        .scan()
        .unwrap()
        .files
        .into_iter()
        .map(|file| (file.relative, file.bytes))
        .collect()
}

fn is_source(path: &Path) -> bool {
    path.extension().is_some_and(|extension| extension == "rs")
}
