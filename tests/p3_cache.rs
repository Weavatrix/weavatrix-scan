#[allow(dead_code)]
#[path = "support/mod.rs"]
mod support;

use weavatrix_scan::{CacheValidationPolicy, ScanOptions, Scanner};

#[test]
fn strict_cache_validation_detects_content_change_under_metadata_collision() {
    let fixture = support::Fixture::new("scan-strict-cache");
    fixture.write("source.rs", "aaaa\n");
    let options = ScanOptions::default().with_extensions(["rs"]);
    let first = Scanner::new(&fixture.root)
        .options(options.clone())
        .scan()
        .unwrap();
    let mut cache = first.to_cache();
    assert_eq!(cache.entries.len(), 1);
    assert!(cache.entries[0].content_fingerprint.starts_with("fp128:"));

    fixture.write("source.rs", "bbbb\n");
    let current_metadata = Scanner::new(&fixture.root)
        .options(options.clone().metadata_only())
        .scan()
        .unwrap();
    cache.entries[0].version = current_metadata.files[0].version;

    let fast = Scanner::new(&fixture.root)
        .options(
            options
                .clone()
                .with_cache_validation(CacheValidationPolicy::Fast),
        )
        .scan_cached(&cache)
        .unwrap();
    assert_eq!(fast.cache.reused_hashes, 1);
    assert_eq!(fast.cache.content_reads, 0);
    assert_eq!(fast.files[0].content_hash, first.files[0].content_hash);

    let strict = Scanner::new(&fixture.root)
        .options(
            options
                .clone()
                .with_cache_validation(CacheValidationPolicy::Strict),
        )
        .scan_cached(&cache)
        .unwrap();
    assert_eq!(strict.cache.reused_hashes, 0);
    assert_eq!(strict.cache.fingerprint_reads, 1);
    assert_eq!(strict.cache.content_reads, 2);
    assert_ne!(strict.files[0].content_hash, first.files[0].content_hash);

    let strict_unchanged = Scanner::new(&fixture.root)
        .options(options.with_cache_validation(CacheValidationPolicy::Strict))
        .scan_cached(&strict.to_cache())
        .unwrap();
    assert_eq!(strict_unchanged.cache.reused_hashes, 1);
    assert_eq!(strict_unchanged.cache.fingerprint_reads, 1);
    assert_eq!(strict_unchanged.cache.content_reads, 1);
    assert_eq!(
        strict_unchanged.files[0].content_hash,
        strict.files[0].content_hash
    );
}
