use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

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
