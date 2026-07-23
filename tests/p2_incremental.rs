#[allow(dead_code)]
mod support;

use support::Fixture;
use weavatrix_scan::{
    DeltaQuality, ErrorPolicy, RepositoryMatch, RepositoryMatcher, ScanDelta, ScanOptions, Scanner,
};

#[test]
fn manifest_delta_classifies_changes_and_only_sound_renames() {
    let fixture = Fixture::new("weavatrix-p2-delta");
    fixture.write(".gitignore", "missing-a\n");
    fixture.write("stable.rs", "fn stable() {}\n");
    fixture.write("modified.rs", "fn before() {}\n");
    fixture.write("removed.rs", "fn removed() {}\n");
    fixture.write("old-name.rs", "fn renamed_unique() {}\n");
    fixture.write("copy/stable.txt", "duplicate\n");
    fixture.write("copy/old.txt", "duplicate\n");
    let options = ScanOptions::default().with_extensions(["rs", "txt"]);
    let previous = Scanner::new(&fixture.root)
        .options(options.clone())
        .scan()
        .unwrap();

    fixture.write(".gitignore", "missing-b\n");
    fixture.write("modified.rs", "fn after_with_more_bytes() {}\n");
    std::fs::remove_file(fixture.root.join("removed.rs")).unwrap();
    std::fs::rename(
        fixture.root.join("old-name.rs"),
        fixture.root.join("new-name.rs"),
    )
    .unwrap();
    std::fs::remove_file(fixture.root.join("copy/old.txt")).unwrap();
    fixture.write("copy/new.txt", "duplicate\n");
    fixture.write("added.rs", "fn added() {}\n");
    let current = Scanner::new(&fixture.root).options(options).scan().unwrap();

    let delta = current.delta_from(&previous);
    assert_eq!(delta.quality, DeltaQuality::ContentHash);
    assert!(delta.selection_inputs_changed);
    assert!(!delta.scan_state_changed);
    assert_eq!(delta.modified.len(), 1);
    assert_eq!(delta.modified[0].current.relative, "modified.rs");
    assert_eq!(delta.renamed.len(), 1);
    assert_eq!(delta.renamed[0].previous.relative, "old-name.rs");
    assert_eq!(delta.renamed[0].current.relative, "new-name.rs");
    assert_eq!(relative_paths(&delta.added), ["added.rs", "copy/new.txt"]);
    assert_eq!(
        relative_paths(&delta.removed),
        ["copy/old.txt", "removed.rs"]
    );
    assert_eq!(delta.unchanged, 2);
    assert!(!delta.is_empty());

    let unchanged = ScanDelta::between(&current, &current);
    assert!(unchanged.is_empty());
    assert_eq!(
        usize::try_from(unchanged.unchanged).unwrap(),
        current.files.len()
    );
}

#[test]
fn metadata_and_partial_deltas_expose_evidence_quality() {
    let fixture = Fixture::new("weavatrix-p2-delta-quality");
    fixture.write("source.rs", "old\n");
    let metadata_options = ScanOptions::default()
        .with_extensions(["rs"])
        .metadata_only();
    let previous = Scanner::new(&fixture.root)
        .options(metadata_options.clone())
        .scan()
        .unwrap();
    fixture.write("source.rs", "longer\n");
    let current = Scanner::new(&fixture.root)
        .options(metadata_options)
        .scan()
        .unwrap();
    let metadata = current.delta_from(&previous);
    assert_eq!(metadata.quality, DeltaQuality::Metadata);
    assert_eq!(metadata.modified.len(), 1);

    let partial = Scanner::new(&fixture.root)
        .options(ScanOptions::default().with_max_entries(Some(0)))
        .scan()
        .unwrap();
    let partial = partial.delta_from(&current);
    assert_eq!(partial.quality, DeltaQuality::Partial);
    assert!(partial.scan_state_changed);
}

#[test]
fn repository_matcher_refreshes_changed_ignore_inputs() {
    let fixture = Fixture::new("weavatrix-p2-matcher-refresh");
    fixture.write(".gitignore", "a.rs\n");
    fixture.write("a.rs", "fn a() {}\n");
    fixture.write("b.rs", "fn b() {}\n");
    let options = ScanOptions::default().with_error_policy(ErrorPolicy::Abort);
    let mut matcher = RepositoryMatcher::with_options(&fixture.root, &options).unwrap();

    assert_eq!(matcher.root(), fixture.root.canonicalize().unwrap());
    assert_eq!(
        matcher.matched("a.rs", false).unwrap(),
        RepositoryMatch::Ignore
    );
    fixture.write(".gitignore", "b.rs\n");
    assert_eq!(
        matcher.matched("a.rs", false).unwrap(),
        RepositoryMatch::Ignore
    );
    assert!(matcher.refresh().unwrap());
    assert_eq!(
        matcher.matched("a.rs", false).unwrap(),
        RepositoryMatch::None
    );
    assert_eq!(
        matcher.matched("b.rs", false).unwrap(),
        RepositoryMatch::Ignore
    );
    assert!(!matcher.refresh().unwrap());

    std::fs::remove_file(fixture.root.join(".gitignore")).unwrap();
    std::fs::create_dir(fixture.root.join(".gitignore")).unwrap();
    assert!(matcher.refresh().is_err());
    assert_eq!(
        matcher.matched("b.rs", false).unwrap(),
        RepositoryMatch::Ignore
    );
}

#[cfg(feature = "serde")]
#[test]
fn scan_delta_round_trips_through_json() {
    let fixture = Fixture::new("weavatrix-p2-delta-serde");
    fixture.write("source.rs", "fn source() {}\n");
    let previous = Scanner::new(&fixture.root).scan().unwrap();
    fixture.write("other.rs", "fn other() {}\n");
    let current = Scanner::new(&fixture.root).scan().unwrap();
    let delta = current.delta_from(&previous);
    let json = serde_json::to_string(&delta).unwrap();
    let decoded: ScanDelta = serde_json::from_str(&json).unwrap();
    assert_eq!(decoded, delta);
}

fn relative_paths(files: &[weavatrix_scan::ScannedFile]) -> Vec<&str> {
    files.iter().map(|file| file.relative.as_str()).collect()
}
