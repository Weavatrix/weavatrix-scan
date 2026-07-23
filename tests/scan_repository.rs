use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};
use weavatrix_scan::{ScanOptions, Scanner, SkipKind};

#[test]
fn scans_deterministically_with_ignore_and_skip_reasons() {
    let fixture = Fixture::new();
    fixture.write(".gitignore", "ignored.rs\n*.tmp\n!important.tmp\n");
    fixture.write("src/lib.rs", "pub fn run() {}\n");
    fixture.write("src/readme.md", "# docs\n");
    fixture.write("ignored.rs", "fn hidden() {}\n");
    fixture.write("debug.tmp", "skip me\n");
    fixture.write("important.tmp", "keep me\n");
    fixture.write("target/generated.rs", "fn generated() {}\n");

    let options = ScanOptions::default().with_extensions(["rs", "tmp"]);
    let first = Scanner::new(&fixture.root)
        .options(options.clone())
        .scan()
        .unwrap();
    let second = Scanner::new(&fixture.root).options(options).scan().unwrap();

    assert_eq!(first.files, second.files);
    assert_eq!(first.skipped, second.skipped);
    assert_eq!(first.revision, second.revision);
    assert_eq!(relative_files(&first), ["important.tmp", "src/lib.rs"]);
    assert!(first.files.iter().all(|file| file.content_hash.is_some()));
    assert!(
        first
            .skipped
            .iter()
            .any(|entry| { entry.relative == "ignored.rs" && entry.kind == SkipKind::Ignored })
    );
    assert!(
        first.skipped.iter().any(|entry| {
            entry.relative == "src/readme.md" && entry.kind == SkipKind::Extension
        })
    );
    assert!(
        first.skipped.iter().any(|entry| {
            entry.relative == "target" && entry.kind == SkipKind::StandardDirectory
        })
    );
}

#[test]
fn reports_oversized_and_binary_files_without_reading_them_as_sources() {
    let fixture = Fixture::new();
    fixture.write("large.rs", "0123456789");
    fs::write(fixture.root.join("binary.rs"), [0, 159, 146, 150]).unwrap();

    let mut options = ScanOptions::default().with_extensions(["rs"]);
    options.max_file_bytes = 4;
    let report = Scanner::new(&fixture.root).options(options).scan().unwrap();

    assert!(report.files.is_empty());
    assert!(
        report
            .skipped
            .iter()
            .any(|entry| { entry.relative == "large.rs" && entry.kind == SkipKind::Oversized })
    );
    assert!(
        report
            .skipped
            .iter()
            .any(|entry| { entry.relative == "binary.rs" && entry.kind == SkipKind::Binary })
    );
}

#[test]
fn metadata_only_skips_content_reads_and_content_hashes() {
    let fixture = Fixture::new();
    fs::write(fixture.root.join("binary.rs"), [0, 159, 146, 150]).unwrap();
    fixture.write("text.rs", "pub fn run() {}\n");

    let report = Scanner::new(&fixture.root)
        .options(
            ScanOptions::default()
                .with_extensions(["rs"])
                .metadata_only(),
        )
        .scan()
        .unwrap();

    assert_eq!(relative_files(&report), ["binary.rs", "text.rs"]);
    assert!(report.files.iter().all(|file| file.content_hash.is_none()));
    assert!(
        report
            .skipped
            .iter()
            .all(|entry| entry.kind != SkipKind::Binary)
    );
}

#[test]
fn parallel_content_inspection_matches_serial_output() {
    let fixture = Fixture::new();
    for index in 0..300 {
        fixture.write(&format!("src/file_{index:03}.rs"), "pub fn run() {}\n");
    }
    fs::write(fixture.root.join("src/binary.rs"), [0, 1, 2, 3]).unwrap();
    let base = ScanOptions::default().with_extensions(["rs"]);

    let serial = Scanner::new(&fixture.root)
        .options(base.clone().with_parallelism(1))
        .scan()
        .unwrap();
    let parallel = Scanner::new(&fixture.root)
        .options(base.with_parallelism(4))
        .scan()
        .unwrap();
    let automatic = Scanner::new(&fixture.root)
        .options(ScanOptions::default().with_extensions(["rs"]))
        .scan()
        .unwrap();

    assert_eq!(serial, parallel);
    assert_eq!(serial, automatic);
}

#[test]
fn skips_symbolic_links() {
    let fixture = Fixture::new();
    fixture.write("target.rs", "pub fn target() {}\n");
    let link = fixture.root.join("link.rs");
    if !create_file_symlink(&fixture.root.join("target.rs"), &link) {
        return;
    }

    let report = Scanner::new(&fixture.root)
        .options(ScanOptions::default().with_extensions(["rs"]))
        .scan()
        .unwrap();

    assert_eq!(relative_files(&report), ["target.rs"]);
    assert!(
        report
            .skipped
            .iter()
            .any(|entry| entry.relative == "link.rs" && entry.kind == SkipKind::Symlink)
    );
}

#[cfg(unix)]
fn create_file_symlink(target: &std::path::Path, link: &std::path::Path) -> bool {
    std::os::unix::fs::symlink(target, link).is_ok()
}

#[cfg(windows)]
fn create_file_symlink(target: &std::path::Path, link: &std::path::Path) -> bool {
    std::os::windows::fs::symlink_file(target, link).is_ok()
}

fn relative_files(report: &weavatrix_scan::ScanReport) -> Vec<String> {
    report
        .files
        .iter()
        .map(|file| file.relative.clone())
        .collect()
}

struct Fixture {
    root: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "weavatrix-scan-test-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(&root).unwrap();
        Self { root }
    }

    fn write(&self, relative: &str, contents: &str) {
        let path = self.root.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, contents).unwrap();
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
