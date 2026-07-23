#[allow(dead_code)]
mod support;

use std::collections::BTreeSet;
use std::path::Path;
use support::Fixture;
use weavatrix_scan::{
    ErrorPolicy, IgnorePolicy, IgnoreSourceKind, ParallelWalker, RepositoryMatch,
    RepositoryMatcher, ScanOptions, Scanner, SkipKind, WalkControl, WalkEvent, WalkOptions, Walker,
};

#[test]
fn local_source_switches_and_require_git_are_independent() {
    let fixture = Fixture::new("weavatrix-p1-sources");
    fixture.write(".gitignore", "git.rs\n");
    fixture.write(".ignore", "dot.rs\n");
    fixture.write(".weavatrixignore", "custom.rs\n");
    for name in ["git.rs", "dot.rs", "custom.rs"] {
        fixture.write(name, "fn source() {}\n");
    }

    let require_git = Scanner::new(&fixture.root)
        .options(
            ScanOptions::default()
                .with_extensions(["rs"])
                .with_ignore_policy(IgnorePolicy::git_compatible()),
        )
        .scan()
        .unwrap();
    assert_eq!(file_names(&require_git), BTreeSet::from(["git.rs"]));
    assert!(
        !require_git
            .ignore_sources
            .iter()
            .any(|source| source.kind == IgnoreSourceKind::GitIgnore)
    );

    fixture.write(".git/HEAD", "ref: refs/heads/main\n");
    let inside_git = Scanner::new(&fixture.root)
        .options(
            ScanOptions::default()
                .with_extensions(["rs"])
                .with_ignore_policy(
                    IgnorePolicy::git_compatible()
                        .with_git_global(false)
                        .with_git_exclude(false),
                ),
        )
        .scan()
        .unwrap();
    assert!(inside_git.files.is_empty());
    assert!(
        inside_git
            .ignore_sources
            .iter()
            .any(|source| source.kind == IgnoreSourceKind::GitIgnore)
    );

    let only_git = IgnorePolicy::repository()
        .with_dot_ignore(false)
        .with_custom_ignore(false);
    let only_git = Scanner::new(&fixture.root)
        .options(
            ScanOptions::default()
                .with_extensions(["rs"])
                .with_ignore_policy(only_git),
        )
        .scan()
        .unwrap();
    assert_eq!(
        file_names(&only_git),
        BTreeSet::from(["custom.rs", "dot.rs"])
    );
}

#[test]
fn override_rules_match_ignore_precedence_and_are_reusable() {
    let fixture = Fixture::new("weavatrix-p1-overrides");
    fixture.write(".gitignore", "src/lib.rs\n");
    fixture.write("src/lib.rs", "fn lib() {}\n");
    fixture.write("src/generated.rs", "fn generated() {}\n");
    fixture.write("src/lib.py", "pass\n");
    fixture.write("README.md", "docs\n");
    fixture.write("target/forced.txt", "forced\n");
    let patterns = [
        "src/**/*.rs",
        "!src/generated.rs",
        "target/",
        "target/forced.txt",
    ];
    let options = ScanOptions::default()
        .with_extensions(["rs", "py", "md"])
        .with_override_rules(patterns)
        .metadata_only();

    let report = Scanner::new(&fixture.root)
        .options(options.clone())
        .scan()
        .unwrap();
    assert_eq!(
        file_names(&report),
        BTreeSet::from(["src/lib.rs", "target/forced.txt"])
    );
    assert!(
        report
            .skipped
            .iter()
            .any(|entry| entry.kind == SkipKind::Override)
    );
    assert!(
        report
            .ignore_sources
            .iter()
            .any(|source| source.kind == IgnoreSourceKind::Override)
    );

    let mut matcher = RepositoryMatcher::with_options(&fixture.root, &options).unwrap();
    assert_eq!(
        matcher.matched("src/lib.rs", false).unwrap(),
        RepositoryMatch::OverrideInclude
    );
    assert_eq!(
        matcher.matched("src/generated.rs", false).unwrap(),
        RepositoryMatch::OverrideIgnore
    );
    assert_eq!(
        matcher.matched("README.md", false).unwrap(),
        RepositoryMatch::OverrideIgnore
    );

    let mut reference = ignore::overrides::OverrideBuilder::new(&fixture.root);
    for pattern in patterns {
        reference.add(pattern).unwrap();
    }
    let reference = reference.build().unwrap();
    for (path, expected) in [
        ("src/lib.rs", false),
        ("src/generated.rs", true),
        ("README.md", true),
        ("target/forced.txt", false),
    ] {
        assert_eq!(
            reference.matched(path, false).is_ignore(),
            expected,
            "path={path}"
        );
    }
}

#[test]
fn invalid_override_rules_follow_the_error_policy() {
    let fixture = Fixture::new("weavatrix-p1-invalid-override");
    fixture.write("lib.rs", "fn lib() {}\n");
    let options = ScanOptions::default().with_override_rules(["{broken"]);
    let continued = Scanner::new(&fixture.root)
        .options(options.clone())
        .scan()
        .unwrap();
    assert!(!continued.complete);
    assert!(
        continued
            .warnings
            .iter()
            .any(|warning| warning.message.contains("<overrides>"))
    );

    let aborted = Scanner::new(&fixture.root)
        .options(options.with_error_policy(ErrorPolicy::Abort))
        .scan();
    assert!(aborted.is_err());
}

#[test]
fn hidden_filter_is_typed_and_explicit_includes_win() {
    let fixture = Fixture::new("weavatrix-p1-hidden");
    fixture.write(".gitignore", "!.visible-hidden.rs\n!.hidden-dir/\n");
    fixture.write(".visible-hidden.rs", "fn visible_hidden() {}\n");
    fixture.write(".drop.rs", "fn drop_me() {}\n");
    fixture.write(".hidden-dir/kept.rs", "fn kept() {}\n");
    fixture.write("visible.rs", "fn visible() {}\n");

    let report = Scanner::new(&fixture.root)
        .options(
            ScanOptions::default()
                .with_extensions(["rs"])
                .with_skip_hidden(true),
        )
        .scan()
        .unwrap();

    assert_eq!(
        file_names(&report),
        BTreeSet::from([".hidden-dir/kept.rs", ".visible-hidden.rs", "visible.rs",])
    );
    assert!(
        report
            .skipped
            .iter()
            .any(|entry| entry.relative == ".drop.rs" && entry.kind == SkipKind::Hidden)
    );
    let options = ScanOptions::default()
        .with_extensions(["rs"])
        .with_skip_hidden(true);
    let mut matcher = RepositoryMatcher::with_options(&fixture.root, &options).unwrap();
    assert_eq!(
        matcher.matched(".visible-hidden.rs", false).unwrap(),
        RepositoryMatch::Include
    );
    assert_eq!(
        matcher.matched(".drop.rs", false).unwrap(),
        RepositoryMatch::Hidden
    );
}

#[test]
fn minimum_depth_matches_serial_parallel_and_nested_scanner_rules() {
    let fixture = Fixture::new("weavatrix-p1-min-depth");
    fixture.write("root.rs", "fn root() {}\n");
    fixture.write("a/.gitignore", "one.rs\n");
    fixture.write("a/one.rs", "fn one() {}\n");
    fixture.write("a/b/two.rs", "fn two() {}\n");
    let walk = WalkOptions::default().with_min_depth(2);

    let serial = Walker::with_options(&fixture.root, walk)
        .unwrap()
        .map(Result::unwrap)
        .map(|entry| entry.relative_path().to_path_buf())
        .collect::<BTreeSet<_>>();
    let parallel = ParallelWalker::new(&fixture.root)
        .options(walk)
        .with_parallelism(4)
        .walk()
        .unwrap()
        .entries
        .into_iter()
        .map(|entry| entry.relative_path().to_path_buf())
        .collect::<BTreeSet<_>>();
    assert_eq!(serial, parallel);
    assert!(serial.iter().all(|path| path != Path::new("root.rs")));

    let visited = std::sync::Arc::new(std::sync::Mutex::new(BTreeSet::new()));
    let visitor_paths = std::sync::Arc::clone(&visited);
    ParallelWalker::new(&fixture.root)
        .options(walk)
        .visit(move |event| {
            if let WalkEvent::Entry(entry) = event {
                visitor_paths
                    .lock()
                    .unwrap()
                    .insert(entry.relative_path().to_path_buf());
            }
            WalkControl::Continue
        })
        .unwrap();
    assert_eq!(*visited.lock().unwrap(), serial);

    let clamped = WalkOptions::default()
        .with_min_depth(3)
        .with_max_depth(Some(1));
    let serial_clamped = Walker::with_options(&fixture.root, clamped)
        .unwrap()
        .map(Result::unwrap)
        .map(|entry| entry.relative_path().to_path_buf())
        .collect::<BTreeSet<_>>();
    let parallel_clamped = ParallelWalker::new(&fixture.root)
        .options(clamped)
        .walk()
        .unwrap()
        .entries
        .into_iter()
        .map(|entry| entry.relative_path().to_path_buf())
        .collect::<BTreeSet<_>>();
    assert_eq!(serial_clamped, parallel_clamped);

    let scan = Scanner::new(&fixture.root)
        .options(
            ScanOptions::default()
                .with_extensions(["rs"])
                .with_min_depth(2),
        )
        .scan()
        .unwrap();
    assert_eq!(file_names(&scan), BTreeSet::from(["a/b/two.rs"]));
}

fn file_names(report: &weavatrix_scan::ScanReport) -> BTreeSet<&str> {
    report
        .files
        .iter()
        .map(|file| file.relative.as_str())
        .collect()
}
