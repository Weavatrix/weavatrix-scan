use super::support::termination_code;
use super::*;
use crate::control::CancellationToken;
use crate::report::{FileVersion, ScanTermination};
use std::fs;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[test]
fn content_inspection_honors_cancel_timeout_and_evidence_modes() {
    let root = fixture("content-stop");
    let first = root.join("first.rs");
    let second = root.join("second.rs");
    fs::write(&first, "fn first() {}\n").unwrap();
    fs::write(&second, "fn second() {}\n").unwrap();
    let files = vec![scanned(first, "first.rs"), scanned(second, "second.rs")];

    let token = CancellationToken::new();
    token.cancel();
    let cancelled = inspect_files(
        files.clone(),
        &ScanOptions::default().with_cancellation(token),
        Instant::now(),
        None,
    )
    .unwrap();
    assert_eq!(cancelled.termination, Some(ScanTermination::Cancelled));
    assert!(cancelled.files.is_empty());
    assert_eq!(cancelled.skipped.len(), 2);

    let timeout = inspect_files(
        files.clone(),
        &ScanOptions::default().with_timeout(Some(Duration::ZERO)),
        Instant::now(),
        None,
    )
    .unwrap();
    assert_eq!(timeout.termination, Some(ScanTermination::Timeout));
    assert_eq!(timeout.skipped.len(), 2);

    let selected = inspect_files(
        files,
        &ScanOptions::default()
            .with_timeout(Some(Duration::ZERO))
            .selected_files_only(),
        Instant::now(),
        None,
    )
    .unwrap();
    assert!(selected.skipped.is_empty());

    assert_eq!(termination_code(ScanTermination::MaxEntries), 0);
    assert_eq!(termination_code(ScanTermination::MaxTotalBytes), 0);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn content_read_errors_continue_or_abort_in_both_read_modes() {
    let root = fixture("content-errors");
    let missing = scanned(root.join("missing.rs"), "missing.rs");

    let continued = inspect_files(
        vec![missing.clone()],
        &ScanOptions::default(),
        Instant::now(),
        None,
    )
    .unwrap();
    assert!(continued.files.is_empty());
    assert_eq!(continued.warnings.len(), 1);
    assert_eq!(continued.skipped[0].kind, SkipKind::IoError);

    let detect_only = ScanOptions {
        hash_file_contents: false,
        ..ScanOptions::default()
    };
    let continued =
        inspect_files(vec![missing.clone()], &detect_only, Instant::now(), None).unwrap();
    assert!(continued.files.is_empty());
    assert_eq!(continued.warnings.len(), 1);

    let aborted = inspect_files(
        vec![missing],
        &ScanOptions::default().with_error_policy(ErrorPolicy::Abort),
        Instant::now(),
        None,
    );
    assert!(aborted.is_err());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn content_changes_between_discovery_and_read_are_typed_or_abort() {
    let root = fixture("content-concurrent");
    let path = root.join("source.rs");
    fs::write(&path, "before\n").unwrap();
    let discovered = scanned(path.clone(), "source.rs");
    fs::write(&path, "changed after discovery\n").unwrap();

    let continued = inspect_files(
        vec![discovered.clone()],
        &ScanOptions::default(),
        Instant::now(),
        None,
    )
    .unwrap();
    assert!(continued.files.is_empty());
    assert_eq!(continued.skipped[0].kind, SkipKind::ConcurrentModification);
    assert_eq!(continued.warnings.len(), 1);
    assert_eq!(continued.cache.content_reads, 1);

    let aborted = inspect_files(
        vec![discovered],
        &ScanOptions::default().with_error_policy(ErrorPolicy::Abort),
        Instant::now(),
        None,
    );
    assert!(matches!(aborted, Err(Error::ConcurrentModification(_))));
    let _ = fs::remove_dir_all(root);
}

fn scanned(absolute: std::path::PathBuf, relative: &str) -> ScannedFile {
    let metadata = fs::metadata(&absolute).ok();
    ScannedFile {
        absolute,
        relative: relative.to_owned(),
        bytes: metadata.as_ref().map_or(0, std::fs::Metadata::len),
        content_hash: None,
        content_fingerprint: None,
        version: metadata
            .as_ref()
            .map_or_else(FileVersion::default, crate::file_version::from_metadata),
        binary_checked: false,
    }
}

fn fixture(name: &str) -> std::path::PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("{name}-{}-{nonce}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    root
}
