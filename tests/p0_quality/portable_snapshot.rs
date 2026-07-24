use super::support::Fixture;
use std::error::Error as _;
use std::fs;
use std::io;
use std::path::PathBuf;
use weavatrix_scan::{
    IgnoreSourceEvidence, IgnoreSourceKind, ScanOptions, ScanWarning, Scanner, SkipKind,
    SkippedEntry, SnapshotEvidence, SnapshotReadError,
};

#[test]
fn portable_report_removes_host_paths_and_free_form_diagnostics() {
    let fixture = Fixture::new("private-repository-name");
    fixture.write(".gitignore", "*.tmp\n");
    fixture.write("src/lib.rs", "pub fn run() {}\n");
    let mut report = Scanner::new(&fixture.root)
        .options(ScanOptions::default().with_extensions(["rs"]))
        .scan()
        .unwrap();
    let private_path = fixture.root.join("outside.ignore");
    report.ignore_sources.push(IgnoreSourceEvidence {
        kind: IgnoreSourceKind::Explicit,
        location: private_path.to_string_lossy().into_owned(),
        content_hash: "ignore-hash".to_owned(),
    });
    report.skipped.push(SkippedEntry {
        relative: "private.tmp".to_owned(),
        kind: SkipKind::IoError,
        detail: Some(format!("failed under {}", fixture.root.display())),
    });
    report.warnings.push(ScanWarning {
        relative: Some("private.tmp".to_owned()),
        message: format!("failed under {}", fixture.root.display()),
    });

    let portable = report.to_portable();
    let rendered = format!("{portable:?}");

    assert!(!rendered.contains("private-repository-name"));
    assert!(!rendered.contains(&fixture.root.to_string_lossy().into_owned()));
    assert_eq!(portable.files[0].relative, "src/lib.rs");
    assert_eq!(
        portable.ignore_sources.last().unwrap().repository_relative,
        None
    );
    assert!(portable.skipped.last().unwrap().detail_hash.is_some());
    assert!(!portable.warnings.last().unwrap().message_hash.is_empty());
    assert_eq!(portable, report.to_portable());
}

#[test]
fn portable_revision_is_independent_of_absolute_root() {
    let first = Fixture::new("portable-first-private-root");
    let second = Fixture::new("portable-second-private-root");
    for fixture in [&first, &second] {
        fixture.write(".gitignore", "*.tmp\n");
        fixture.write("src/lib.rs", "pub fn run() {}\n");
    }
    let options = ScanOptions::default().with_extensions(["rs"]);
    let mut first_report = Scanner::new(&first.root)
        .options(options.clone())
        .scan()
        .unwrap();
    let mut second_report = Scanner::new(&second.root).options(options).scan().unwrap();
    first_report.ignore_sources.push(IgnoreSourceEvidence {
        kind: IgnoreSourceKind::Explicit,
        location: first.root.join("external.ignore").display().to_string(),
        content_hash: "same-external-rules".to_owned(),
    });
    second_report.ignore_sources.push(IgnoreSourceEvidence {
        kind: IgnoreSourceKind::Explicit,
        location: second.root.join("external.ignore").display().to_string(),
        content_hash: "same-external-rules".to_owned(),
    });
    let first = first_report.to_portable();
    let second = second_report.to_portable();

    assert_eq!(first.revision, second.revision);
    assert_eq!(first.files, second.files);
}

#[test]
fn snapshot_provider_reads_hash_verified_content_and_rejects_stale_files() {
    let fixture = Fixture::new("snapshot-provider");
    fixture.write("src/lib.rs", "pub fn one() {}\n");
    let report = Scanner::new(&fixture.root)
        .options(ScanOptions::default().with_extensions(["rs"]))
        .scan()
        .unwrap();
    let provider = report.content_provider().unwrap();
    let content = provider.read("src/lib.rs").unwrap();

    assert_eq!(content.bytes, b"pub fn one() {}\n");
    assert_eq!(content.evidence, SnapshotEvidence::Sha256);
    assert!(matches!(
        provider.read("src/missing.rs"),
        Err(SnapshotReadError::UnknownFile(relative)) if relative == "src/missing.rs"
    ));
    assert!(matches!(
        provider.read_bounded("src/lib.rs", 4),
        Err(SnapshotReadError::LimitExceeded { relative, .. }) if relative == "src/lib.rs"
    ));

    fs::write(fixture.root.join("src/lib.rs"), "pub fn two() {}\n").unwrap();
    assert!(matches!(
        provider.read("src/lib.rs"),
        Err(SnapshotReadError::Stale(relative)) if relative == "src/lib.rs"
    ));
}

#[test]
fn snapshot_provider_supports_version_evidence_and_rejects_path_escape() {
    let fixture = Fixture::new("snapshot-provider-version");
    fixture.write("src/lib.rs", "pub fn run() {}\n");
    let mut report = Scanner::new(&fixture.root)
        .options(
            ScanOptions::default()
                .with_extensions(["rs"])
                .metadata_only(),
        )
        .scan()
        .unwrap();
    let provider = report.content_provider().unwrap();
    let content = provider.read("src/lib.rs").unwrap();
    assert_eq!(content.evidence, SnapshotEvidence::FileVersion);

    report.files[0].absolute = fixture.root.join("../escaped.rs");
    assert!(matches!(
        report.content_provider(),
        Err(SnapshotReadError::InvalidReport { .. })
    ));
}

#[test]
fn snapshot_errors_and_report_validation_are_explicit() {
    let fixture = Fixture::new("snapshot-validation");
    fixture.write("a.rs", "fn a() {}\n");
    fixture.write("b.rs", "fn b() {}\n");
    let report = Scanner::new(&fixture.root)
        .options(ScanOptions::default().with_extensions(["rs"]))
        .scan()
        .unwrap();

    let errors = [
        SnapshotReadError::InvalidReport {
            relative: None,
            reason: "invalid root",
        },
        SnapshotReadError::UnknownFile("unknown.rs".to_owned()),
        SnapshotReadError::LimitExceeded {
            relative: "large.rs".to_owned(),
            bytes: 2,
            max_bytes: 1,
        },
        SnapshotReadError::Stale("stale.rs".to_owned()),
    ];
    for error in errors {
        assert!(!error.to_string().is_empty());
        assert!(error.source().is_none());
    }
    let io_error = SnapshotReadError::Io {
        relative: "io.rs".to_owned(),
        source: io::Error::new(io::ErrorKind::PermissionDenied, "denied"),
    };
    assert!(io_error.to_string().contains("io.rs"));
    assert!(io_error.source().is_some());

    let mut relative_root = report.clone();
    relative_root.root = PathBuf::from("relative");
    assert!(matches!(
        relative_root.content_provider(),
        Err(SnapshotReadError::InvalidReport { relative: None, .. })
    ));

    let mut unsorted = report.clone();
    unsorted.files.reverse();
    assert!(matches!(
        unsorted.content_provider(),
        Err(SnapshotReadError::InvalidReport { .. })
    ));

    let mut mismatched = report.clone();
    mismatched.files[0].relative = "wrong.rs".to_owned();
    assert!(matches!(
        mismatched.content_provider(),
        Err(SnapshotReadError::InvalidReport { .. })
    ));

    let mut unverifiable = report.clone();
    unverifiable.files[0].content_hash = None;
    unverifiable.files[0].version.modified_ns = None;
    assert!(matches!(
        unverifiable.content_provider(),
        Err(SnapshotReadError::InvalidReport { .. })
    ));

    let provider = report.content_provider().unwrap();
    fs::remove_file(fixture.root.join("a.rs")).unwrap();
    assert!(matches!(
        provider.read("a.rs"),
        Err(SnapshotReadError::Stale(relative)) if relative == "a.rs"
    ));
}
