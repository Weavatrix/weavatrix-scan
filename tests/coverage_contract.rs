use std::error::Error as _;
use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};
use weavatrix_scan::{Error, ErrorPolicy, ScanOptions, Scanner, scan_repository};

#[test]
fn public_errors_expose_paths_messages_and_sources() {
    let fixture = Fixture::new("weavatrix-scan-errors");
    let missing = fixture.root.join("missing");
    let io_error = Scanner::new(&missing).scan().unwrap_err();
    assert!(matches!(io_error, Error::Io { .. }));
    assert!(io_error.to_string().contains("missing"));
    assert!(io_error.source().is_some());

    let file = fixture.root.join("not-a-directory");
    fs::write(&file, "text").unwrap();
    let invalid = Scanner::new(&file).scan().unwrap_err();
    assert!(matches!(invalid, Error::InvalidRoot(_)));
    assert!(invalid.to_string().contains("not a directory"));
    assert!(invalid.source().is_none());
}

#[test]
fn custom_ignore_uppercase_extensions_and_default_entrypoint_work() {
    let fixture = Fixture::new("weavatrix-scan-options");
    fs::write(fixture.root.join(".analysisignore"), "skip.rs\n").unwrap();
    fs::write(fixture.root.join("skip.rs"), "fn skip() {}\n").unwrap();
    fs::write(fixture.root.join("KEEP.RS"), "fn keep() {}\n").unwrap();
    fs::write(fixture.root.join("extensionless"), "text\n").unwrap();

    let report = Scanner::new(&fixture.root)
        .options(
            ScanOptions::default()
                .with_extensions([".rs"])
                .with_ignore_files([".analysisignore"])
                .with_parallelism(0),
        )
        .scan()
        .unwrap();

    assert_eq!(report.files.len(), 1);
    assert_eq!(report.files[0].relative, "KEEP.RS");

    let empty = Fixture::new("weavatrix-scan-empty");
    let report = scan_repository(&empty.root).unwrap();
    assert!(report.files.is_empty());
    assert!(!report.revision.is_empty());
}

#[test]
fn unreadable_ignore_entry_is_reported_as_a_warning() {
    let fixture = Fixture::new("weavatrix-scan-warning");
    fs::create_dir(fixture.root.join(".gitignore")).unwrap();
    fs::write(
        fixture.root.join(".gitignore/nested.rs"),
        "fn nested() {}\n",
    )
    .unwrap();
    fs::create_dir(fixture.root.join(".weavatrixignore")).unwrap();
    fs::write(
        fixture.root.join(".weavatrixignore/nested.rs"),
        "fn nested() {}\n",
    )
    .unwrap();

    let report = Scanner::new(&fixture.root)
        .options(ScanOptions::default().with_extensions(["rs"]))
        .scan()
        .unwrap();

    assert_eq!(report.warnings.len(), 2);
    assert!(
        report.warnings[0]
            .message
            .contains("could not load ignore rules")
    );
    assert!(!report.complete);

    let error = Scanner::new(&fixture.root)
        .options(
            ScanOptions::default()
                .with_extensions(["rs"])
                .with_error_policy(ErrorPolicy::Abort),
        )
        .scan()
        .unwrap_err();
    assert!(matches!(error, Error::Io { .. }));
}

#[test]
fn malformed_ignore_rules_are_partial_evidence_or_abort() {
    let fixture = Fixture::new("weavatrix-scan-invalid-ignore");
    fs::write(fixture.root.join(".gitignore"), "{broken\n*.tmp\n").unwrap();
    fs::write(fixture.root.join("hidden.tmp"), "ignored\n").unwrap();
    fs::write(fixture.root.join("visible.rs"), "fn visible() {}\n").unwrap();

    let report = Scanner::new(&fixture.root)
        .options(
            ScanOptions::default()
                .with_extensions(["rs", "tmp"])
                .metadata_only(),
        )
        .scan()
        .unwrap();
    assert_eq!(report.files.len(), 1);
    assert_eq!(report.files[0].relative, "visible.rs");
    assert!(!report.complete);
    assert!(
        report
            .warnings
            .iter()
            .any(|warning| warning.message.contains(".gitignore"))
    );

    let error = Scanner::new(&fixture.root)
        .options(
            ScanOptions::default()
                .with_extensions(["rs", "tmp"])
                .with_error_policy(ErrorPolicy::Abort),
        )
        .scan()
        .unwrap_err();
    assert!(matches!(error, Error::Io { .. }));
}

#[test]
fn ignore_matching_supports_bom_and_configurable_case_insensitivity() {
    let fixture = Fixture::new("weavatrix-scan-ignore-case");
    fs::write(
        fixture.root.join(".gitignore"),
        "\u{feff}DROP.RS\nignored.tmp\n",
    )
    .unwrap();
    fs::write(fixture.root.join("drop.rs"), "fn drop() {}\n").unwrap();
    fs::write(fixture.root.join("ignored.tmp"), "ignored\n").unwrap();
    fs::write(fixture.root.join("keep.rs"), "fn keep() {}\n").unwrap();

    let report = Scanner::new(&fixture.root)
        .options(
            ScanOptions::default()
                .with_extensions(["rs", "tmp"])
                .with_ignore_case_insensitive(true)
                .metadata_only(),
        )
        .scan()
        .unwrap();
    let files = report
        .files
        .iter()
        .map(|file| file.relative.as_str())
        .collect::<Vec<_>>();
    assert_eq!(files, ["keep.rs"]);
    assert!(report.complete);
}

struct Fixture {
    root: std::path::PathBuf,
}

impl Fixture {
    fn new(prefix: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("{prefix}-{}-{nonce}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        Self { root }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
