// Ignore-policy quality cases.
use super::support::Fixture;
use std::collections::BTreeSet;
use std::path::Path;
use weavatrix_scan::{
    IgnorePolicy, IgnoreSourceKind, RepositoryMatcher, ScanOptions, Scanner, StandardSkips,
};

#[test]
fn repository_ignore_sources_have_stable_precedence_and_provenance() {
    let fixture = Fixture::new("weavatrix-p0-ignore-precedence");
    fixture.write(".gitignore", "a.rs\nb.rs\n");
    fixture.write(".ignore", "!a.rs\n!b.rs\n");
    fixture.write(".weavatrixignore", "b.rs\n");
    fixture.write("a.rs", "fn a() {}\n");
    fixture.write("b.rs", "fn b() {}\n");

    let report = Scanner::new(&fixture.root)
        .options(ScanOptions::default().with_extensions(["rs"]))
        .scan()
        .unwrap();

    assert_eq!(
        report
            .files
            .iter()
            .map(|file| file.relative.as_str())
            .collect::<Vec<_>>(),
        ["a.rs"]
    );
    assert!(report.portable);
    assert_eq!(
        report
            .ignore_sources
            .iter()
            .map(|source| source.kind)
            .collect::<Vec<_>>(),
        [
            IgnoreSourceKind::GitIgnore,
            IgnoreSourceKind::DotIgnore,
            IgnoreSourceKind::Custom,
        ]
    );
}

#[test]
fn matcher_reuses_parent_git_exclude_and_explicit_rules() {
    let fixture = Fixture::new("weavatrix-p0-matcher");
    fixture.write(".git/HEAD", "ref: refs/heads/main\n");
    fixture.write(".git/info/exclude", "*.cache\n");
    fixture.write(".selection-ignore", "*.generated.rs\n");
    fixture.write("packages/app/src/main.rs", "fn main() {}\n");
    fixture.write("packages/app/src/private.cache", "cache\n");
    fixture.write("packages/app/src/code.generated.rs", "generated\n");
    fixture.write(".gitignore", "*.secret\n");
    fixture.write("packages/app/src/token.secret", "secret\n");

    let root = fixture.root.join("packages/app");
    let policy = IgnorePolicy::repository()
        .with_parent_rules(true)
        .with_git_exclude(true)
        .with_explicit_file(fixture.root.join(".selection-ignore"));
    let options = ScanOptions::default()
        .with_ignore_policy(policy)
        .with_extensions(["rs", "cache", "secret"])
        .metadata_only();
    let mut matcher = RepositoryMatcher::with_options(&root, &options).unwrap();

    assert!(!matcher.is_ignored("src/main.rs", false).unwrap());
    assert!(matcher.is_ignored("src/private.cache", false).unwrap());
    assert!(matcher.is_ignored("src/code.generated.rs", false).unwrap());
    assert!(matcher.is_ignored("src/token.secret", false).unwrap());
    assert!(
        matcher
            .is_ignored(fixture.root.join("sibling.rs"), false)
            .is_err()
    );
    assert!(matcher.prepare_directory(&fixture.root).is_err());
    assert!(!matcher.portable());
    assert!(
        matcher
            .sources()
            .iter()
            .any(|source| source.kind == IgnoreSourceKind::GitExclude)
    );
    assert!(
        matcher
            .sources()
            .iter()
            .any(|source| source.kind == IgnoreSourceKind::Explicit)
    );
}

#[test]
fn ignore_inputs_participate_in_revision_even_when_selection_is_unchanged() {
    let fixture = Fixture::new("weavatrix-p0-ignore-revision");
    fixture.write(".gitignore", "missing-a.rs\n");
    fixture.write("lib.rs", "fn lib() {}\n");
    let options = ScanOptions::default().with_extensions(["rs"]);
    let first = Scanner::new(&fixture.root)
        .options(options.clone())
        .scan()
        .unwrap();
    fixture.write(".gitignore", "missing-b.rs\n");
    let second = Scanner::new(&fixture.root).options(options).scan().unwrap();

    assert_eq!(first.files, second.files);
    assert_ne!(first.revision, second.revision);
}

#[test]
fn repository_matcher_respects_disabled_standard_skips_independently() {
    let fixture = Fixture::new("weavatrix-p0-matcher-selection");
    fixture.write("target/generated.rs", "fn generated() {}\n");
    let mut options = ScanOptions::default().metadata_only();
    options.standard_skips = StandardSkips::Disabled;
    let mut matcher = RepositoryMatcher::with_options(&fixture.root, &options).unwrap();
    assert!(!matcher.is_ignored("target/generated.rs", false).unwrap());
    assert!(matcher.is_ignored("../outside.rs", false).is_err());
}

#[test]
fn ignore_policy_constructors_and_default_matcher_are_usable() {
    let fixture = Fixture::new("weavatrix-p0-policy-constructors");
    fixture.write("custom.rules", "*.generated\n");
    fixture.write("source.rs", "fn source() {}\n");

    let repository = IgnorePolicy::repository().with_explicit_file("custom.rules");
    assert!(!repository.parent_rules);
    assert_eq!(repository.explicit_files.len(), 1);
    let compatible = IgnorePolicy::git_compatible();
    assert!(compatible.parent_rules);
    assert!(compatible.git_exclude);
    assert!(compatible.git_global);
    let none = IgnorePolicy::none();
    assert!(!none.git_ignore);
    assert!(!none.dot_ignore);
    assert!(!none.custom_ignore);
    assert!(!none.git_exclude);
    assert!(!none.git_global);
    assert!(none.explicit_files.is_empty());
    let options = ScanOptions::default().with_standard_skips(StandardSkips::Disabled);
    assert_eq!(options.standard_skips, StandardSkips::Disabled);

    let mut matcher = RepositoryMatcher::new(&fixture.root).unwrap();
    assert!(!matcher.is_ignored("source.rs", false).unwrap());
    let missing = fixture.root.join("missing");
    assert!(RepositoryMatcher::new(missing).is_err());
}

#[test]
fn git_exclude_and_repository_sources_match_ignore_crate_precedence() {
    let fixture = Fixture::new("weavatrix-p0-git-source-parity");
    fixture.write(".git/HEAD", "ref: refs/heads/main\n");
    fixture.write(".git/info/exclude", "*.rs\n");
    fixture.write(".gitignore", "!git.rs\n");
    fixture.write(".ignore", "!dot.rs\n");
    fixture.write(".weavatrixignore", "!custom.rs\n");
    for name in ["git.rs", "dot.rs", "custom.rs", "excluded.rs"] {
        fixture.write(name, "fn source() {}\n");
    }
    let policy = IgnorePolicy::repository().with_git_exclude(true);
    let mut options = ScanOptions::default()
        .with_ignore_policy(policy)
        .with_extensions(["rs"])
        .metadata_only();
    options.standard_skips = StandardSkips::Disabled;
    let ours = Scanner::new(&fixture.root).options(options).scan().unwrap();
    let ours = ours
        .files
        .iter()
        .map(|file| file.relative.clone())
        .collect::<BTreeSet<_>>();

    let mut reference = ignore::WalkBuilder::new(&fixture.root);
    reference
        .add_custom_ignore_filename(".weavatrixignore")
        .git_global(false)
        .git_exclude(true)
        .hidden(false)
        .parents(false)
        .require_git(false);
    let reference = reference
        .build()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_some_and(|kind| kind.is_file()))
        .filter_map(|entry| {
            let path = entry.into_path();
            (path.extension().and_then(|value| value.to_str()) == Some("rs")).then(|| {
                path.strip_prefix(&fixture.root)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/")
            })
        })
        .collect::<BTreeSet<_>>();

    assert_eq!(ours, reference);
}

#[test]
fn repository_ignore_symlinks_are_not_followed() {
    let fixture = Fixture::new("weavatrix-p0-ignore-link");
    let outside = Fixture::new("weavatrix-p0-ignore-link-target");
    outside.write("rules", "secret.rs\n");
    fixture.write("secret.rs", "fn secret() {}\n");
    if !create_file_symlink(
        &outside.root.join("rules"),
        &fixture.root.join(".gitignore"),
    ) {
        return;
    }

    let report = Scanner::new(&fixture.root)
        .options(ScanOptions::default().with_extensions(["rs"]))
        .scan()
        .unwrap();

    assert_eq!(report.files[0].relative, "secret.rs");
    assert!(!report.complete);
    assert!(
        report
            .warnings
            .iter()
            .any(|warning| warning.message.contains("symbolic link"))
    );
}

#[cfg(unix)]
fn create_file_symlink(target: &Path, link: &Path) -> bool {
    std::os::unix::fs::symlink(target, link).is_ok()
}

#[cfg(windows)]
fn create_file_symlink(target: &Path, link: &Path) -> bool {
    std::os::windows::fs::symlink_file(target, link).is_ok()
}
