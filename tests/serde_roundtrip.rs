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

    let mut legacy = serde_json::to_value(&report).unwrap();
    legacy.as_object_mut().unwrap().remove("complete");
    let legacy: weavatrix_scan::ScanReport = serde_json::from_value(legacy).unwrap();
    assert!(legacy.complete);

    let _ = std::fs::remove_dir_all(fixture);
}

#[cfg(target_os = "linux")]
#[test]
fn non_utf8_report_paths_round_trip_losslessly() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt as _;

    let fixture = std::env::temp_dir().join(format!(
        "weavatrix-scan-serde-native-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&fixture).unwrap();
    let name = OsString::from_vec(vec![
        b'n', b'a', b't', b'i', b'v', b'e', 0x80, b'.', b'r', b's',
    ]);
    std::fs::write(fixture.join(&name), "fn run() {}\n").unwrap();

    let report = Scanner::new(&fixture)
        .options(ScanOptions::default().with_extensions(["rs"]))
        .scan()
        .unwrap();
    let json = serde_json::to_string(&report).unwrap();
    let decoded: weavatrix_scan::ScanReport = serde_json::from_str(&json).unwrap();
    assert_eq!(report, decoded);

    let _ = std::fs::remove_dir_all(fixture);
}

#[cfg(windows)]
#[test]
fn non_unicode_report_paths_round_trip_losslessly() {
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt as _;

    let fixture = std::env::temp_dir().join(format!(
        "weavatrix-scan-serde-native-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&fixture).unwrap();
    let name = OsString::from_wide(&[
        u16::from(b'n'),
        u16::from(b'a'),
        u16::from(b't'),
        u16::from(b'i'),
        u16::from(b'v'),
        u16::from(b'e'),
        0xd800,
        u16::from(b'.'),
        u16::from(b'r'),
        u16::from(b's'),
    ]);
    if std::fs::write(fixture.join(&name), "fn run() {}\n").is_err() {
        let _ = std::fs::remove_dir_all(fixture);
        return;
    }

    let report = Scanner::new(&fixture)
        .options(ScanOptions::default().with_extensions(["rs"]))
        .scan()
        .unwrap();
    let json = serde_json::to_string(&report).unwrap();
    let decoded: weavatrix_scan::ScanReport = serde_json::from_str(&json).unwrap();
    assert_eq!(report, decoded);

    let _ = std::fs::remove_dir_all(fixture);
}
