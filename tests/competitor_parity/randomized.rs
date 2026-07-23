use super::*;

#[test]
fn randomized_hierarchical_ignore_files_match_ignore_crate() {
    const ROOT_PATTERNS: &[&str] = &[
        "*.tmp",
        "*.log",
        "!keep.tmp",
        "/root-?.rs",
        "docs/",
        "!docs/",
        "**/cache/*.tmp",
        "[ab].rs",
        "nested/*/drop.rs",
        "name[0-9].txt",
        "escaped\\ name.txt",
        "generated/**",
    ];
    const LOCAL_PATTERNS: &[&str] = &[
        "local-?.rs",
        "!local-a.rs",
        "*.generated.ts",
        "!keep.generated.ts",
        "/anchored.rs",
        "cache/",
    ];

    for seed in 0..12_u64 {
        let fixture = support::Fixture::new(&format!("weavatrix-scan-random-ignore-{seed}"));
        for directory in ["", "src", "docs", "nested/a", "nested/b", "src/cache"] {
            for name in [
                "a.rs",
                "b.rs",
                "z.rs",
                "drop.rs",
                "keep.tmp",
                "drop.tmp",
                "trace.log",
                "name7.txt",
                "escaped name.txt",
                "local-a.rs",
                "local-b.rs",
                "keep.generated.ts",
                "drop.generated.ts",
                "anchored.rs",
            ] {
                let path = if directory.is_empty() {
                    name.to_owned()
                } else {
                    format!("{directory}/{name}")
                };
                fixture.write(&path, "text\n");
            }
        }
        fixture.write("root-a.rs", "text\n");
        fixture.write("generated/deep/output.rs", "text\n");

        let mut state = seed.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let root_rules = randomized_rules(ROOT_PATTERNS, &mut state, 18);
        let local_rules = randomized_rules(LOCAL_PATTERNS, &mut state, 10);
        let custom_rules = randomized_rules(ROOT_PATTERNS, &mut state, 8);
        fixture.write(".gitignore", format!("{root_rules}\n"));
        fixture.write("src/.gitignore", format!("{local_rules}\n"));
        fixture.write(".weavatrixignore", format!("{custom_rules}\n"));

        let extensions = ["rs", "tmp", "log", "txt", "ts"];
        let mut options = ScanOptions::default()
            .with_extensions(extensions)
            .metadata_only();
        options.standard_skips = StandardSkips::Disabled;
        let ours = Scanner::new(&fixture.root).options(options).scan().unwrap();
        let ours_files = relative_set(ours.files.iter().map(|file| file.relative.as_str()));
        let ignore_files = ignore_crate_files(&fixture.root, &extensions);
        assert_eq!(ours_files, ignore_files, "differential seed {seed}");
    }
}

#[test]
fn deep_tree_selection_matches_ignore_without_recursive_traversal() {
    let fixture = support::Fixture::new("weavatrix-scan-deep-parity");
    let mut directory = fixture.root.clone();
    for _ in 0..70 {
        directory.push("d");
        std::fs::create_dir(&directory).unwrap();
    }
    std::fs::write(directory.join("deep.rs"), "fn deep() {}\n").unwrap();
    let mut options = ScanOptions::default()
        .with_extensions(["rs"])
        .metadata_only();
    options.standard_skips = StandardSkips::Disabled;
    let ours = Scanner::new(&fixture.root).options(options).scan().unwrap();
    let ours_files = relative_set(ours.files.iter().map(|file| file.relative.as_str()));
    assert_eq!(ours_files, ignore_crate_files(&fixture.root, &["rs"]));
}
