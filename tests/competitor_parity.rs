mod support;

use ignore::WalkBuilder;
use std::collections::BTreeSet;
use std::path::Path;
use support::build_scan_corpus;
use weavatrix_scan::{ScanOptions, Scanner, SkipKind, StandardSkips};

#[test]
fn matches_ignore_crate_for_text_source_selection_when_policies_align() {
    let fixture = build_scan_corpus("weavatrix-scan-parity", 3, 4);
    let extensions = ["rs", "go", "ts", "tmp"];
    let mut options = ScanOptions::default().with_extensions(extensions);
    options.standard_skips = StandardSkips::Disabled;

    let ours = Scanner::new(&fixture.root).options(options).scan().unwrap();
    let ours_files = relative_set(ours.files.iter().map(|file| file.relative.as_str()));
    let ignore_files = ignore_crate_files(&fixture.root, &extensions);

    assert_eq!(ours_files, ignore_files);
    assert!(!ours.revision.is_empty());
    assert!(
        ours.skipped
            .iter()
            .any(|entry| { entry.relative == "binary.rs" && entry.kind == SkipKind::Binary })
    );
}

#[test]
fn builds_a_richer_scan_manifest_than_plain_walkers() {
    let fixture = build_scan_corpus("weavatrix-scan-manifest", 2, 2);
    let report = Scanner::new(&fixture.root)
        .options(ScanOptions::default().with_extensions(["rs", "go", "ts"]))
        .scan()
        .unwrap();

    assert!(report.files.iter().all(|file| {
        file.absolute.is_absolute() && !file.relative.contains('\\') && file.content_hash.is_some()
    }));
    assert!(
        report.skipped.iter().any(|entry| {
            entry.relative == "target" && entry.kind == SkipKind::StandardDirectory
        })
    );
    assert!(
        report
            .skipped
            .iter()
            .any(|entry| { entry.relative == "ignored_dir" && entry.kind == SkipKind::Ignored })
    );
    assert!(
        report
            .skipped
            .iter()
            .any(|entry| { entry.relative == "README.md" && entry.kind == SkipKind::Extension })
    );
    assert!(
        report
            .skipped
            .iter()
            .any(|entry| { entry.relative == "binary.rs" && entry.kind == SkipKind::Binary })
    );
}

#[test]
fn matches_ignore_for_anchored_nested_and_character_class_patterns() {
    let fixture = support::Fixture::new("weavatrix-scan-pattern-parity");
    fixture.write(
        ".gitignore",
        "*.log\n!keep.log\n/root-only.rs\ndocs/\n**/cache/*.tmp\n[ab].rs\nescaped\\ name.txt\n",
    );
    fixture.write("src/.gitignore", "local-?.rs\n!local-a.rs\n");
    for path in [
        "keep.log",
        "drop.log",
        "root-only.rs",
        "src/root-only.rs",
        "docs/guide.rs",
        "src/cache/drop.tmp",
        "src/cache/keep.rs",
        "a.rs",
        "z.rs",
        "escaped name.txt",
        "src/local-a.rs",
        "src/local-b.rs",
    ] {
        fixture.write(path, "text\n");
    }
    let extensions = ["rs", "log", "tmp", "txt"];
    let mut options = ScanOptions::default()
        .with_extensions(extensions)
        .metadata_only();
    options.standard_skips = StandardSkips::Disabled;

    let ours = Scanner::new(&fixture.root).options(options).scan().unwrap();
    let ours_files = relative_set(ours.files.iter().map(|file| file.relative.as_str()));
    let ignore_files = ignore_crate_files(&fixture.root, &extensions);

    assert_eq!(ours_files, ignore_files);
}

fn ignore_crate_files(root: &Path, extensions: &[&str]) -> BTreeSet<String> {
    let extensions = extensions.iter().copied().collect::<BTreeSet<_>>();
    let mut builder = WalkBuilder::new(root);
    builder
        .add_custom_ignore_filename(".weavatrixignore")
        .git_global(false)
        .git_exclude(false)
        .hidden(false)
        .parents(false)
        .require_git(false);

    builder
        .build()
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .file_type()
                .is_some_and(|file_type| file_type.is_file())
        })
        .filter_map(|entry| {
            let path = entry.into_path();
            let extension = path.extension()?.to_str()?.to_ascii_lowercase();
            extensions.contains(extension.as_str()).then_some(path)
        })
        .filter(|path| !std::fs::read(path).unwrap_or_default().contains(&0))
        .map(|path| {
            path.strip_prefix(root)
                .unwrap()
                .components()
                .map(|component| component.as_os_str().to_string_lossy())
                .collect::<Vec<_>>()
                .join("/")
        })
        .collect()
}

fn relative_set<'a>(items: impl IntoIterator<Item = &'a str>) -> BTreeSet<String> {
    items.into_iter().map(str::to_owned).collect()
}
