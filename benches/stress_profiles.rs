mod support;

use jwalk::WalkDir as JWalkDir;
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use support::{BenchmarkCase, measure, measure_group, print_measurement};
use walkdir::WalkDir;
use weavatrix_scan::{ParallelWalker, ScanOptions, Scanner, WalkEntry, WalkOptions, Walker};

fn main() {
    let skewed = StressFixture::skewed();
    let small = StressFixture::small();
    let first_touch = Instant::now();
    let first_count = parallel_files(&skewed.root).len();
    println!(
        "mode=first-touch library=weavatrix-parallel files={first_count} elapsed_ms={:.3}",
        first_touch.elapsed().as_secs_f64() * 1_000.0
    );
    benchmark_skewed(&skewed);
    benchmark_small_parallel(&small);
    benchmark_small_scanner(&small);
    benchmark_deep(&StressFixture::deep());
    benchmark_large_incremental(&StressFixture::large());
    benchmark_bounded_handles(&skewed);
}

fn benchmark_small_parallel(fixture: &StressFixture) {
    let expected = serial_files(&fixture.root);
    let mut cases = vec![
        BenchmarkCase::new("weavatrix-parallel", || {
            checked(&parallel_files(&fixture.root), &expected)
        }),
        BenchmarkCase::new("weavatrix-parallel-8", || {
            checked(&parallel_files_with_workers(&fixture.root, 8), &expected)
        }),
        BenchmarkCase::new("weavatrix-parallel-16", || {
            checked(&parallel_files_with_workers(&fixture.root, 16), &expected)
        }),
    ];
    let results = measure_group(&mut cases);
    for (case, result) in cases.iter().zip(results) {
        print_measurement("small-raw", case.name, &result);
    }
}

fn benchmark_small_scanner(fixture: &StressFixture) {
    let expected = scan_files(&fixture.root, 1);
    let mut cases = vec![
        BenchmarkCase::new("weavatrix-scan-serial", || scan_files(&fixture.root, 1)),
        BenchmarkCase::new("weavatrix-scan-4", || scan_files(&fixture.root, 4)),
        BenchmarkCase::new("weavatrix-scan-8", || scan_files(&fixture.root, 8)),
        BenchmarkCase::new("weavatrix-scan-16", || scan_files(&fixture.root, 16)),
    ];
    let results = measure_group(&mut cases);
    for (case, result) in cases.iter().zip(results) {
        assert_eq!(result.count, expected);
        print_measurement("small-scan", case.name, &result);
    }
}

fn benchmark_skewed(fixture: &StressFixture) {
    let expected = serial_files(&fixture.root);
    let mut cases = vec![
        BenchmarkCase::new("weavatrix-parallel", || {
            checked(&parallel_files(&fixture.root), &expected)
        }),
        BenchmarkCase::new("weavatrix-parallel-8", || {
            checked(&parallel_files_with_workers(&fixture.root, 8), &expected)
        }),
        BenchmarkCase::new("weavatrix-parallel-16", || {
            checked(&parallel_files_with_workers(&fixture.root, 16), &expected)
        }),
        BenchmarkCase::new("jwalk", || checked(&jwalk_files(&fixture.root), &expected)),
        BenchmarkCase::new("walkdir", || {
            checked(&walkdir_files(&fixture.root), &expected)
        }),
    ];
    let results = measure_group(&mut cases);
    for (case, result) in cases.iter().zip(results) {
        print_measurement("skewed-raw", case.name, &result);
    }
}

fn benchmark_deep(fixture: &StressFixture) {
    let expected = walkdir_files(&fixture.root);
    let mut cases = vec![
        BenchmarkCase::new("weavatrix-walker", || {
            checked(&serial_files(&fixture.root), &expected)
        }),
        BenchmarkCase::new("walkdir", || {
            checked(&walkdir_files(&fixture.root), &expected)
        }),
    ];
    let results = measure_group(&mut cases);
    for (case, result) in cases.iter().zip(results) {
        print_measurement("deep-raw", case.name, &result);
    }
}

fn benchmark_large_incremental(fixture: &StressFixture) {
    let options = ScanOptions::default()
        .with_extensions(["rs"])
        .selected_files_only();
    let previous = Scanner::new(&fixture.root)
        .options(options.clone())
        .scan()
        .unwrap();
    let full = measure(|| {
        Scanner::new(&fixture.root)
            .options(options.clone())
            .scan()
            .unwrap()
            .files
            .len()
    });
    let incremental = measure(|| {
        let report = Scanner::new(&fixture.root)
            .options(options.clone())
            .scan_incremental(&previous)
            .unwrap();
        assert_eq!(report.cache.content_reads, 0);
        report.files.len()
    });
    print_measurement("large-content", "weavatrix-full", &full);
    print_measurement("large-content", "weavatrix-incremental", &incremental);
}

fn benchmark_bounded_handles(fixture: &StressFixture) {
    let result = measure(|| {
        ParallelWalker::new(&fixture.root)
            .options(WalkOptions::default().with_max_open(4))
            .walk()
            .unwrap()
            .entries
            .into_iter()
            .filter(WalkEntry::is_file)
            .count()
    });
    print_measurement("bounded-handles-max-open-4", "weavatrix-parallel", &result);
}

type Paths = Vec<PathBuf>;

fn checked(actual: &Paths, expected: &Paths) -> usize {
    assert_eq!(actual, expected);
    actual.len()
}

fn serial_files(root: &Path) -> Paths {
    sorted(
        root,
        Walker::new(root)
            .unwrap()
            .filter_map(Result::ok)
            .filter(WalkEntry::is_file)
            .map(WalkEntry::into_path),
    )
}

fn parallel_files(root: &Path) -> Paths {
    parallel_files_with_workers(root, 0)
}

fn parallel_files_with_workers(root: &Path, workers: usize) -> Paths {
    let mut files = ParallelWalker::new(root)
        .with_parallelism(workers)
        .walk()
        .unwrap()
        .entries
        .into_iter()
        .filter(WalkEntry::is_file)
        .map(|entry| entry.relative_path().to_path_buf())
        .collect::<Vec<_>>();
    files.sort_unstable();
    files
}

fn jwalk_files(root: &Path) -> Paths {
    sorted(
        root,
        JWalkDir::new(root)
            .sort(false)
            .skip_hidden(false)
            .into_iter()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_type().is_file())
            .map(|entry| entry.path()),
    )
}

fn walkdir_files(root: &Path) -> Paths {
    sorted(
        root,
        WalkDir::new(root)
            .into_iter()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_type().is_file())
            .map(walkdir::DirEntry::into_path),
    )
}

fn scan_files(root: &Path, workers: usize) -> usize {
    Scanner::new(root)
        .options(
            ScanOptions::default()
                .with_extensions(["rs"])
                .metadata_only()
                .selected_files_only()
                .with_parallelism(workers),
        )
        .scan()
        .unwrap()
        .files
        .len()
}

fn sorted(root: &Path, files: impl Iterator<Item = PathBuf>) -> Paths {
    let mut files = files
        .map(|path| path.strip_prefix(root).unwrap().to_path_buf())
        .collect::<Vec<_>>();
    files.sort_unstable();
    files
}

struct StressFixture {
    root: PathBuf,
}

impl StressFixture {
    fn new(name: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("{name}-{}-{nonce}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        Self { root }
    }

    fn skewed() -> Self {
        let fixture = Self::new("weavatrix-bench-skewed");
        for directory in 0..64 {
            for file in 0..40 {
                fixture.write(
                    &format!("single/module_{directory:02}/file_{file:02}.rs"),
                    b"fn run() {}\n",
                );
            }
        }
        fixture
    }

    fn small() -> Self {
        let fixture = Self::new("weavatrix-bench-small");
        for directory in 0..4 {
            for file in 0..8 {
                fixture.write(
                    &format!("src/module_{directory}/file_{file}.rs"),
                    b"fn run() {}\n",
                );
            }
        }
        fixture
    }

    fn deep() -> Self {
        let fixture = Self::new("weavatrix-bench-deep");
        let mut directory = fixture.root.clone();
        for depth in 0..60 {
            directory.push("d");
            std::fs::create_dir(&directory).unwrap();
            for file in 0..128 {
                std::fs::write(
                    directory.join(format!("{depth}-{file}.rs")),
                    b"fn run() {}\n",
                )
                .unwrap();
            }
        }
        fixture
    }

    fn large() -> Self {
        let fixture = Self::new("weavatrix-bench-large");
        let contents = vec![b'x'; 256 * 1024];
        for file in 0..48 {
            fixture.write(&format!("src/file_{file:02}.rs"), &contents);
        }
        fixture
    }

    fn write(&self, relative: &str, contents: &[u8]) {
        let path = self.root.join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, contents).unwrap();
    }
}

impl Drop for StressFixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}
