#[allow(dead_code)]
mod support;

use std::collections::BTreeSet;
use support::Fixture;
use weavatrix_scan::{
    RepositoryMatch, ScanOptions, Scanner, SelectionDisposition, SelectionMatcher, SkipKind, Walker,
};

#[test]
fn complete_selection_matcher_matches_scanner_manifest() {
    let fixture = Fixture::new("weavatrix-selection-parity");
    fixture.write(".gitignore", "ignored.rs\n");
    fixture.write("ignored.rs", "ignored");
    fixture.write(".hidden.rs", "hidden");
    fixture.write("src/lib.rs", "lib");
    fixture.write("src/readme.txt", "text");
    fixture.write("src/large.rs", "123456789");
    fixture.write("target/generated.rs", "generated");
    fixture.write("target/forced.txt", "forced");
    fixture.write(".private/secret.rs", "secret");
    fixture.write("nested/deep/more/file.rs", "deep");

    let mut options = ScanOptions::default()
        .with_extensions(["rs"])
        .with_override_rules(["target/", "target/forced.txt"])
        .with_skip_hidden(true)
        .with_max_depth(Some(3))
        .metadata_only();
    options.max_file_bytes = 8;

    let report = Scanner::new(&fixture.root)
        .options(options.clone())
        .scan()
        .unwrap();
    let expected = report
        .files
        .iter()
        .map(|file| file.relative.clone())
        .collect::<BTreeSet<_>>();

    let mut matcher = SelectionMatcher::with_options(&fixture.root, &options).unwrap();
    let mut walker = Walker::with_options(&fixture.root, options.walk.with_metadata(true)).unwrap();
    let mut actual = BTreeSet::new();
    while let Some(entry) = walker.next() {
        let entry = entry.unwrap();
        let decision = matcher.matched_entry(&entry).unwrap();
        if decision.is_selected() {
            actual.insert(
                entry
                    .relative_path()
                    .to_string_lossy()
                    .replace(std::path::MAIN_SEPARATOR, "/"),
            );
        }
        if matches!(decision.disposition(), SelectionDisposition::Skipped(_)) && entry.is_dir() {
            walker.skip_current_dir();
        }
    }
    assert_eq!(actual, expected);

    assert_eq!(
        matcher.matched("ignored.rs").unwrap().skip_kind(),
        Some(SkipKind::Override)
    );
    let forced = matcher.matched("target/forced.txt").unwrap();
    assert!(forced.is_selected());
    assert_eq!(forced.repository_match(), RepositoryMatch::OverrideInclude);
    assert!(
        matcher
            .clone()
            .matched("target/forced.txt")
            .unwrap()
            .is_selected()
    );
    assert_eq!(
        matcher
            .matched("nested/deep/more/file.rs")
            .unwrap()
            .skip_kind(),
        Some(SkipKind::MaxDepth)
    );
}

#[test]
fn standalone_matching_applies_ancestor_and_file_policies() {
    let fixture = Fixture::new("weavatrix-selection-standalone");
    fixture.write(".gitignore", "ignored.rs\n");
    fixture.write("ignored.rs", "ignored");
    fixture.write(".hidden.rs", "hidden");
    fixture.write("src/readme.txt", "text");
    fixture.write("src/large.rs", "123456789");
    fixture.write("target/generated.rs", "generated");
    fixture.write(".private/secret.rs", "secret");
    let mut path_options = ScanOptions::default()
        .with_extensions(["rs"])
        .with_skip_hidden(true)
        .with_max_depth(Some(3))
        .metadata_only();
    path_options.max_file_bytes = 8;
    let mut ignore_only = SelectionMatcher::with_options(&fixture.root, &path_options).unwrap();
    assert_eq!(
        ignore_only.matched("ignored.rs").unwrap().skip_kind(),
        Some(SkipKind::Ignored)
    );
    assert_eq!(
        ignore_only.matched(".hidden.rs").unwrap().skip_kind(),
        Some(SkipKind::Hidden)
    );
    assert_eq!(
        ignore_only.matched("src/readme.txt").unwrap().skip_kind(),
        Some(SkipKind::Extension)
    );
    assert_eq!(
        ignore_only.matched("src/large.rs").unwrap().skip_kind(),
        Some(SkipKind::Oversized)
    );
    assert_eq!(
        ignore_only
            .matched("target/generated.rs")
            .unwrap()
            .skip_kind(),
        Some(SkipKind::StandardDirectory)
    );
    assert_eq!(
        ignore_only
            .matched(".private/secret.rs")
            .unwrap()
            .skip_kind(),
        Some(SkipKind::Hidden)
    );
}

#[test]
fn matcher_normalization_is_lossless_and_rejects_escape() {
    let fixture = Fixture::new("weavatrix-selection-normalize");
    fixture.write("src/lib.rs", "lib");
    let repository = weavatrix_scan::RepositoryMatcher::new(&fixture.root).unwrap();

    assert_eq!(
        repository.normalize("src/lib.rs").unwrap(),
        std::path::PathBuf::from("src/lib.rs")
    );
    assert!(repository.normalize("../outside.rs").is_err());
    assert!(
        repository
            .normalize(fixture.root.join("..").join("outside.rs"))
            .is_err()
    );
}
