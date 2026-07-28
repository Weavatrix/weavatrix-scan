#[allow(dead_code)]
mod support;

use support::Fixture;
use weavatrix_scan::{ScanOptions, Scanner, SkipKind};

#[test]
fn full_and_compact_reports_share_a_path_free_summary() {
    let fixture = Fixture::new("scan-summary");
    fixture.write(".gitignore", "ignored.rs\n");
    fixture.write("src/lib.rs", "pub fn run() {}\n");
    fixture.write("ignored.rs", "fn ignored() {}\n");

    let options = ScanOptions::default()
        .with_extensions(["rs"])
        .metadata_only();
    let full = Scanner::new(&fixture.root)
        .options(options.clone())
        .scan()
        .unwrap();
    let compact = Scanner::new(&fixture.root)
        .options(options)
        .scan_compact()
        .unwrap();
    let summary = full.summary();

    assert_eq!(summary, compact.summary());
    assert_eq!(summary.selected_files, 1);
    assert_eq!(summary.selected_bytes, 16);
    assert_eq!(summary.hashed_files, 0);
    assert_eq!(summary.recorded_skips, full.skipped.len());
    assert_eq!(summary.skipped_by_kind[&SkipKind::Ignored], 1);
    assert!(summary.complete);
    assert!(summary.portable);
    assert!(
        !summary
            .to_string()
            .contains(fixture.root.to_string_lossy().as_ref())
    );
    assert_eq!(
        summary.to_string(),
        "files=1 bytes=16 hashed=0 binary_checked=0 skipped=2 warnings=0 \
         ignore_sources=1 complete=true portable=true"
    );
}

#[cfg(feature = "serde")]
#[test]
fn summary_has_a_stable_serde_round_trip() {
    let fixture = Fixture::new("scan-summary-serde");
    fixture.write("src/lib.rs", "pub fn run() {}\n");
    let report = Scanner::new(&fixture.root)
        .options(
            ScanOptions::default()
                .with_extensions(["rs"])
                .metadata_only(),
        )
        .scan()
        .unwrap();
    let summary = report.summary();
    let encoded = serde_json::to_string(&summary).unwrap();
    let decoded = serde_json::from_str(&encoded).unwrap();

    assert_eq!(summary, decoded);
}
