// Native-path walker cases.
use super::*;

#[cfg(target_os = "linux")]
#[test]
fn non_utf8_paths_remain_lossless_and_get_collision_free_manifest_names() {
    use std::ffi::OsString;
    use std::os::unix::ffi::{OsStrExt as _, OsStringExt as _};

    let fixture = Fixture::new("weavatrix-walker-non-utf8");
    let name = OsString::from_vec(vec![b'b', b'a', b'd', 0x80, b'.', b'r', b's']);
    fs::write(fixture.root.join(&name), "fn run() {}\n").unwrap();

    let native = Walker::new(&fixture.root)
        .unwrap()
        .map(Result::unwrap)
        .find(|entry| entry.depth() == 1)
        .unwrap();
    assert_eq!(native.file_name().as_bytes(), name.as_bytes());

    let report = Scanner::new(&fixture.root)
        .options(ScanOptions::default().with_extensions(["rs"]))
        .scan()
        .unwrap();
    assert_eq!(report.files.len(), 1);
    assert_eq!(report.files[0].relative, "bad%80.rs");
}

#[cfg(windows)]
#[test]
fn non_unicode_windows_paths_are_escaped_without_replacement() {
    use std::ffi::OsString;
    use std::os::windows::ffi::{OsStrExt as _, OsStringExt as _};

    let fixture = Fixture::new("weavatrix-walker-non-unicode");
    let name = OsString::from_wide(&[
        u16::from(b'b'),
        u16::from(b'a'),
        u16::from(b'd'),
        0xd800,
        u16::from(b'.'),
        u16::from(b'r'),
        u16::from(b's'),
    ]);
    if fs::write(fixture.root.join(&name), "fn run() {}\n").is_err() {
        return;
    }

    let native = Walker::new(&fixture.root)
        .unwrap()
        .map(Result::unwrap)
        .find(|entry| entry.depth() == 1)
        .unwrap();
    assert_eq!(
        native.file_name().encode_wide().collect::<Vec<_>>(),
        name.encode_wide().collect::<Vec<_>>()
    );

    let report = Scanner::new(&fixture.root)
        .options(ScanOptions::default().with_extensions(["rs"]))
        .scan()
        .unwrap();
    assert_eq!(report.files.len(), 1);
    assert_eq!(report.files[0].relative, "bad%uD800.rs");
}

#[cfg(unix)]
#[test]
fn scanner_continues_after_permission_errors_with_partial_typed_evidence() {
    use std::os::unix::fs::PermissionsExt as _;

    let fixture = Fixture::new("weavatrix-scanner-permission");
    fixture.write("blocked/hidden.rs", "fn hidden() {}\n");
    fixture.write("visible.rs", "fn visible() {}\n");
    let blocked = fixture.root.join("blocked");
    fs::set_permissions(&blocked, fs::Permissions::from_mode(0o000)).unwrap();
    let report = Scanner::new(&fixture.root)
        .options(ScanOptions::default().with_extensions(["rs"]))
        .scan()
        .unwrap();
    fs::set_permissions(&blocked, fs::Permissions::from_mode(0o700)).unwrap();

    if report.complete {
        return;
    }
    assert_eq!(report.files[0].relative, "visible.rs");
    assert!(
        report
            .skipped
            .iter()
            .any(|entry| entry.kind == SkipKind::IoError)
    );
}
