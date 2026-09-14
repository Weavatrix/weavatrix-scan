use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};
use weavatrix_scan::{ScanOptions, ScanReport, Scanner, WatchPlan};

pub struct Fixture {
    pub root: PathBuf,
}

impl Fixture {
    pub fn new(prefix: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("{prefix}-{}-{nonce}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        Self { root }
    }

    pub fn write(&self, relative: &str, contents: impl AsRef<[u8]>) {
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

#[allow(dead_code)]
pub fn relatives(report: &ScanReport) -> Vec<&str> {
    report
        .files
        .iter()
        .map(|file| file.relative.as_str())
        .collect()
}

#[allow(dead_code)]
pub fn watch_matches(
    fixture: &Fixture,
    options: &ScanOptions,
    previous: &ScanReport,
    plan: &WatchPlan,
) -> ScanReport {
    let updated = Scanner::new(&fixture.root)
        .options(options.clone())
        .scan_watch_plan(previous, plan)
        .unwrap();
    let full = Scanner::new(&fixture.root)
        .options(options.clone())
        .scan()
        .unwrap();
    assert_eq!(relatives(&updated), relatives(&full));
    assert_eq!(
        updated
            .files
            .iter()
            .map(|file| (&file.relative, file.bytes, &file.content_hash))
            .collect::<Vec<_>>(),
        full.files
            .iter()
            .map(|file| (&file.relative, file.bytes, &file.content_hash))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        updated
            .skipped
            .iter()
            .map(|entry| (&entry.relative, entry.kind, &entry.detail))
            .collect::<Vec<_>>(),
        full.skipped
            .iter()
            .map(|entry| (&entry.relative, entry.kind, &entry.detail))
            .collect::<Vec<_>>()
    );
    assert_eq!(updated.complete, full.complete);
    assert_eq!(updated.revision, full.revision);
    assert_eq!(updated.descriptor, full.descriptor);
    updated
}

pub fn build_scan_corpus(prefix: &str, directories: usize, files_per_language: usize) -> Fixture {
    let fixture = Fixture::new(prefix);
    fixture.write(
        ".gitignore",
        "ignored.rs\nignored_dir/\n*.tmp\n!important.tmp\n",
    );
    fixture.write(".weavatrixignore", "secret.yaml\n");
    fixture.write("ignored.rs", "fn hidden() {}\n");
    fixture.write("ignored_dir/hidden.rs", "fn hidden() {}\n");
    fixture.write("important.tmp", "keep me\n");
    fixture.write("scratch.tmp", "skip me\n");
    fixture.write("secret.yaml", "token: hidden\n");
    fixture.write("target/generated.rs", "fn generated() {}\n");
    fixture.write("README.md", "# docs\n");
    fixture.write("binary.rs", [0, 159, 146, 150]);

    for directory_index in 0..directories {
        for file_index in 0..files_per_language {
            let base = format!("src/module_{directory_index:03}/file_{file_index:03}");
            fixture.write(
                &format!("{base}.rs"),
                "pub fn run() { helper(); }\npub fn helper() {}\n",
            );
            fixture.write(&format!("{base}.go"), "package main\nfunc run() {}\n");
            fixture.write(
                &format!("{base}.ts"),
                "export function run() { return 1 }\n",
            );
        }
    }
    fixture
}
