use super::*;

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
fn low_level_walkers_match_walkdir_and_jwalk_on_raw_entries() {
    let fixture = build_scan_corpus("weavatrix-raw-walker-parity", 12, 6);
    let ours = Walker::with_options(&fixture.root, WalkOptions::default())
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
        .into_iter()
        .map(|entry| entry.relative_path().to_path_buf())
        .collect::<BTreeSet<_>>();
    let walkdir = WalkDir::new(&fixture.root)
        .follow_links(false)
        .into_iter()
        .map(Result::unwrap)
        .map(|entry| {
            entry
                .path()
                .strip_prefix(&fixture.root)
                .unwrap()
                .to_path_buf()
        })
        .collect::<BTreeSet<_>>();
    assert_eq!(ours, walkdir);

    let parallel = ParallelWalker::new(&fixture.root)
        .options(WalkOptions::default().with_max_open(3))
        .with_parallelism(8)
        .walk()
        .unwrap();
    assert!(parallel.errors.is_empty());
    let parallel = parallel
        .entries
        .into_iter()
        .map(|entry| entry.relative_path().to_path_buf())
        .collect::<BTreeSet<_>>();
    let jwalk = JWalkDir::new(&fixture.root)
        .sort(false)
        .skip_hidden(false)
        .into_iter()
        .map(Result::unwrap)
        .map(|entry| {
            entry
                .path()
                .strip_prefix(&fixture.root)
                .unwrap()
                .to_path_buf()
        })
        .collect::<BTreeSet<_>>();
    assert_eq!(parallel, jwalk);
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
