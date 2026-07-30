use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

pub(super) struct StressFixture {
    pub(super) root: PathBuf,
}

impl StressFixture {
    pub(super) fn new(name: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("{name}-{}-{nonce}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        Self { root }
    }

    pub(super) fn skewed() -> Self {
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

    pub(super) fn small() -> Self {
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

    pub(super) fn deep() -> Self {
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

    pub(super) fn large() -> Self {
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
