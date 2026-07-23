#![cfg(feature = "serde")]

use weavatrix_scan::{ScanOptions, Scanner};

#[test]
fn scan_report_round_trips_through_json() {
    let fixture = std::env::temp_dir().join(format!("weavatrix-scan-serde-{}", std::process::id()));
    std::fs::create_dir_all(&fixture).unwrap();
    std::fs::write(fixture.join("lib.rs"), "pub fn run() {}\n").unwrap();

    let report = Scanner::new(&fixture)
        .options(ScanOptions::default().with_extensions(["rs"]))
        .scan()
        .unwrap();
    let json = serde_json::to_string(&report).unwrap();
    let decoded = serde_json::from_str(&json).unwrap();

    assert_eq!(report, decoded);
    let _ = std::fs::remove_dir_all(fixture);
}
