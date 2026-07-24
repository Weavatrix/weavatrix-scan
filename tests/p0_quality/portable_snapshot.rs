use super::support::Fixture;
use std::fs;
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
