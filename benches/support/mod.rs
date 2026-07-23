#![allow(dead_code)]

use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub const DIRECTORIES: usize = 80;
pub const FILES_PER_LANGUAGE: usize = 25;
pub const EXTENSIONS: &[&str] = &["rs", "go", "ts"];
pub const SOURCE_FILES: usize = DIRECTORIES * FILES_PER_LANGUAGE * EXTENSIONS.len();
pub const RAW_FILES: usize = SOURCE_FILES + 4;
pub const IGNORE_AWARE_FILES: usize = SOURCE_FILES + 1;

pub struct Measurement {
    pub count: usize,
    pub median: Duration,
    pub minimum: Duration,
}

pub struct BenchmarkCase<'a> {
    pub name: &'static str,
    operation: Box<dyn FnMut() -> usize + 'a>,
}

impl<'a> BenchmarkCase<'a> {
    pub fn new(name: &'static str, operation: impl FnMut() -> usize + 'a) -> Self {
        Self {
            name,
            operation: Box::new(operation),
        }
    }
}

pub fn measure(mut operation: impl FnMut() -> usize) -> Measurement {
    const WARMUPS: usize = 2;
    const RUNS: usize = 11;

    for _ in 0..WARMUPS {
        std::hint::black_box(operation());
    }
    let mut samples = Vec::with_capacity(RUNS);
    let expected = operation();
    for _ in 0..RUNS {
        let start = Instant::now();
        let count = std::hint::black_box(operation());
        assert_eq!(count, expected);
        samples.push(start.elapsed());
    }
    samples.sort_unstable();
    Measurement {
        count: expected,
        median: samples[RUNS / 2],
        minimum: samples[0],
    }
}

pub fn measure_group(cases: &mut [BenchmarkCase<'_>]) -> Vec<Measurement> {
    const WARMUPS: usize = 2;
    const RUNS: usize = 11;

    let mut counts = vec![None; cases.len()];
    for _ in 0..WARMUPS {
        for (index, case) in cases.iter_mut().enumerate() {
            let count = std::hint::black_box((case.operation)());
            assert!(counts[index].is_none_or(|expected| expected == count));
            counts[index] = Some(count);
        }
    }
    let mut samples = vec![Vec::with_capacity(RUNS); cases.len()];
    for round in 0..RUNS {
        for offset in 0..cases.len() {
            let index = (round + offset) % cases.len();
            let start = Instant::now();
            let count = std::hint::black_box((cases[index].operation)());
            assert_eq!(Some(count), counts[index]);
            samples[index].push(start.elapsed());
        }
    }
    samples
        .into_iter()
        .zip(counts)
        .map(|(mut samples, count)| {
            samples.sort_unstable();
            Measurement {
                count: count.unwrap(),
                median: samples[RUNS / 2],
                minimum: samples[0],
            }
        })
        .collect()
}

pub fn print_measurement(mode: &str, library: &str, measurement: &Measurement) {
    println!(
        "mode={mode} library={library} files={} median_ms={:.3} min_ms={:.3}",
        measurement.count,
        measurement.median.as_secs_f64() * 1_000.0,
        measurement.minimum.as_secs_f64() * 1_000.0
    );
}

pub struct Fixture {
    pub root: PathBuf,
}

impl Fixture {
    pub fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "weavatrix-scan-bench-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join(".gitignore"),
            "ignored.rs\nignored_dir/\ntarget/\n*.tmp\n",
        )
        .unwrap();
        fs::write(root.join(".weavatrixignore"), "secret.yaml\n").unwrap();
        write(&root, "ignored.rs", "fn hidden() {}\n");
        write(&root, "ignored_dir/hidden.rs", "fn hidden() {}\n");
        write(&root, "target/generated.rs", "fn generated() {}\n");
        write(&root, "debug.tmp", "skip me\n");
        write(&root, "secret.yaml", "token: hidden\n");
        fs::write(root.join("binary.rs"), [0, 159, 146, 150]).unwrap();

        for directory_index in 0..DIRECTORIES {
            let directory = root.join(format!("module_{directory_index:03}"));
            fs::create_dir_all(&directory).unwrap();
            for file_index in 0..FILES_PER_LANGUAGE {
                for extension in EXTENSIONS {
                    fs::write(
                        directory.join(format!("file_{file_index:03}.{extension}")),
                        source_for(extension),
                    )
                    .unwrap();
                }
            }
        }
        Self { root }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn write(root: &std::path::Path, relative: &str, contents: &str) {
    let path = root.join(relative);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, contents).unwrap();
}

fn source_for(extension: &str) -> &'static str {
    match extension {
        "rs" => "pub fn run() { helper(); }\npub fn helper() {}\n",
        "go" => "package main\nfunc run() {}\n",
        "ts" => "export function run() { return 1 }\n",
        _ => "",
    }
}
