// Snapshot-integrity quality cases.
use super::support::Fixture;
use std::thread;
use std::time::Duration;
use weavatrix_scan::{ScanOptions, Scanner};

#[test]
fn strong_hashes_are_reused_and_same_size_changes_are_detected() {
    let fixture = Fixture::new("weavatrix-p0-integrity");
    fixture.write("source.rs", "aaaa\n");
    let options = ScanOptions::default().with_extensions(["rs"]);
    let previous = Scanner::new(&fixture.root)
        .options(options.clone())
        .scan()
        .unwrap();

    assert_eq!(previous.cache.content_reads, 1);
    assert_eq!(previous.cache.reused_hashes, 0);
    assert!(
        previous.files[0]
            .content_hash
            .as_deref()
            .is_some_and(|hash| hash.starts_with("sha256:"))
    );

    let unchanged = Scanner::new(&fixture.root)
        .options(options.clone())
        .scan_incremental(&previous)
        .unwrap();
    assert_eq!(unchanged.revision, previous.revision);
    assert_eq!(unchanged.cache.content_reads, 0);
    assert_eq!(unchanged.cache.reused_hashes, 1);

    thread::sleep(Duration::from_millis(20));
    fixture.write("source.rs", "bbbb\n");
    let changed = Scanner::new(&fixture.root)
        .options(options)
        .scan_incremental(&unchanged)
        .unwrap();
    assert_eq!(changed.cache.content_reads, 1);
    assert_eq!(changed.cache.reused_hashes, 0);
    assert_ne!(
        changed.files[0].content_hash,
        unchanged.files[0].content_hash
    );
    let delta = changed.delta_from(&unchanged);
    assert_eq!(delta.modified.len(), 1);
    assert_eq!(delta.modified[0].current.relative, "source.rs");
}

#[test]
fn cache_from_another_root_is_never_reused() {
    let first = Fixture::new("weavatrix-p0-cache-first");
    let second = Fixture::new("weavatrix-p0-cache-second");
    first.write("source.rs", "same\n");
    second.write("source.rs", "same\n");
    let options = ScanOptions::default().with_extensions(["rs"]);
    let previous = Scanner::new(&first.root)
        .options(options.clone())
        .scan()
        .unwrap();
    let current = Scanner::new(&second.root)
        .options(options)
        .scan_incremental(&previous)
        .unwrap();

    assert_eq!(current.cache.reused_hashes, 0);
    assert_eq!(current.cache.content_reads, 1);
}
