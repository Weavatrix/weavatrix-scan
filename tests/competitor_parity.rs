mod support;

use ignore::WalkBuilder;
use jwalk::WalkDir as JWalkDir;
use std::collections::BTreeSet;
use std::path::Path;
use support::build_scan_corpus;
use walkdir::WalkDir;
#[cfg(unix)]
use weavatrix_scan::WalkSkipReason;
use weavatrix_scan::{
    ParallelWalker, ScanOptions, Scanner, SkipKind, StandardSkips, WalkOptions, Walker,
};

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

#[cfg(target_os = "linux")]
#[test]
fn non_utf8_paths_and_symlink_loops_have_lossless_differential_evidence() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt as _;

    let fixture = support::Fixture::new("weavatrix-scan-native-parity");
    let native_name = OsString::from_vec(vec![
        b'n', b'a', b't', b'i', b'v', b'e', 0x80, b'.', b'r', b's',
    ]);
    std::fs::write(fixture.root.join(&native_name), "fn native() {}\n").unwrap();
    fixture.write("src/lib.rs", "fn run() {}\n");
    std::os::unix::fs::symlink(&fixture.root, fixture.root.join("src/back")).unwrap();

    let ours = Walker::with_options(
        &fixture.root,
        WalkOptions::default().with_follow_links(true),
    )
    .unwrap()
    .collect::<Result<Vec<_>, _>>()
    .unwrap();
    assert!(ours.iter().any(|entry| {
        entry.relative_path() == Path::new("src/back")
            && entry.skip_reason() == Some(WalkSkipReason::SymlinkLoop)
    }));
    let ours_files = ours
        .iter()
        .filter(|entry| entry.is_file())
        .map(|entry| entry.relative_path().to_path_buf())
        .collect::<BTreeSet<_>>();

    let mut builder = WalkBuilder::new(&fixture.root);
    builder
        .follow_links(true)
        .hidden(false)
        .parents(false)
        .require_git(false);
    let mut ignore_files = BTreeSet::new();
    let mut loop_errors = 0;
    for item in builder.build() {
        match item {
            Ok(entry) if entry.file_type().is_some_and(|kind| kind.is_file()) => {
                ignore_files.insert(
                    entry
                        .path()
                        .strip_prefix(&fixture.root)
                        .unwrap()
                        .to_path_buf(),
                );
            }
            Ok(_) => {}
            Err(_) => loop_errors += 1,
        }
    }
    assert_eq!(ours_files, ignore_files);
    assert!(loop_errors > 0);
}

#[cfg(unix)]
#[test]
fn permission_errors_are_non_fatal_for_both_walkers() {
    use std::os::unix::fs::PermissionsExt as _;

    let fixture = support::Fixture::new("weavatrix-scan-permission-parity");
    fixture.write("blocked/hidden.rs", "fn hidden() {}\n");
    fixture.write("visible.rs", "fn visible() {}\n");
    let blocked = fixture.root.join("blocked");
    std::fs::set_permissions(&blocked, std::fs::Permissions::from_mode(0o000)).unwrap();

    let mut options = ScanOptions::default()
        .with_extensions(["rs"])
        .metadata_only();
    options.standard_skips = StandardSkips::Disabled;
    let ours = Scanner::new(&fixture.root).options(options).scan().unwrap();
    let mut builder = WalkBuilder::new(&fixture.root);
    builder
        .hidden(false)
        .parents(false)
        .require_git(false)
        .git_ignore(false)
        .git_exclude(false)
        .git_global(false);
    let mut ignore_files = BTreeSet::new();
    let mut ignore_errors = 0;
    for item in builder.build() {
        match item {
            Ok(entry) if entry.file_type().is_some_and(|kind| kind.is_file()) => {
                ignore_files.insert(
                    entry
                        .path()
                        .strip_prefix(&fixture.root)
                        .unwrap()
                        .to_string_lossy()
                        .replace('\\', "/"),
                );
            }
            Ok(_) => {}
            Err(_) => ignore_errors += 1,
        }
    }
    std::fs::set_permissions(&blocked, std::fs::Permissions::from_mode(0o700)).unwrap();

    if ours.complete {
        return;
    }
    let ours_files = relative_set(ours.files.iter().map(|file| file.relative.as_str()));
    assert_eq!(ours_files, ignore_files);
    assert!(ignore_errors > 0);
    assert!(
        ours.skipped
            .iter()
            .any(|entry| entry.kind == SkipKind::IoError)
    );
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

fn randomized_rules(patterns: &[&str], state: &mut u64, count: usize) -> String {
    (0..count)
        .map(|_| {
            *state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            let modulus = u64::try_from(patterns.len()).expect("pattern count fits u64");
            let index = usize::try_from(*state % modulus).expect("remainder fits usize");
            patterns[index]
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn relative_set<'a>(items: impl IntoIterator<Item = &'a str>) -> BTreeSet<String> {
    items.into_iter().map(str::to_owned).collect()
}
