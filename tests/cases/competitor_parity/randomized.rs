// Randomized hierarchical ignore-parity cases.
use super::*;
use std::io::Write as _;
use std::process::{Command, Stdio};

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
        "**/deep/**/drop-*.rs",
        "\\#literal.rs",
        "\\!literal.rs",
        "trailing-space\\ ",
        "question-?.[ch]",
        "!nested/**/keep.rs",
    ];
    const LOCAL_PATTERNS: &[&str] = &[
        "local-?.rs",
        "!local-a.rs",
        "*.generated.ts",
        "!keep.generated.ts",
        "/anchored.rs",
        "cache/",
    ];

    for seed in 0..96_u64 {
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
                "#literal.rs",
                "!literal.rs",
                "keep.rs",
                "question-a.c",
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
fn randomized_gitignore_selection_matches_git_check_ignore() {
    const PATTERNS: &[&str] = &[
        "*.tmp",
        "*.log",
        "!keep.tmp",
        "/root-?.rs",
        "docs/",
        "!docs/keep.rs",
        "**/cache/*.tmp",
        "[ab].rs",
        "nested/*/drop.rs",
        "name[0-9].txt",
        "escaped\\ name.txt",
        "generated/**",
        "**/deep/**/drop-*.rs",
        "\\#literal.rs",
        "\\!literal.rs",
        "question-?.[ch]",
        "!nested/**/keep.rs",
    ];
    const PATHS: &[&str] = &[
        "a.rs",
        "b.rs",
        "z.rs",
        "root-a.rs",
        "src/root-a.rs",
        "keep.tmp",
        "drop.tmp",
        "trace.log",
        "docs/keep.rs",
        "docs/drop.rs",
        "src/cache/drop.tmp",
        "src/cache/keep.rs",
        "nested/a/drop.rs",
        "nested/a/keep.rs",
        "nested/deep/a/drop-one.rs",
        "name7.txt",
        "escaped name.txt",
        "#literal.rs",
        "!literal.rs",
        "question-a.c",
    ];

    for seed in 0..16_u64 {
        let fixture = support::Fixture::new(&format!("scan-git-differential-{seed}"));
        if !git_init(&fixture.root) {
            return;
        }
        for path in PATHS {
            fixture.write(path, "text\n");
        }
        let mut state = seed.wrapping_add(0x517c_c1b7_2722_0a95);
        fixture.write(
            ".gitignore",
            format!("{}\n", randomized_rules(PATTERNS, &mut state, 28)),
        );
        let mut options = ScanOptions::default()
            .with_extensions(["rs", "tmp", "log", "txt", "c"])
            .metadata_only();
        options.standard_skips = StandardSkips::Disabled;
        options.ignore_policy = weavatrix_scan::IgnorePolicy::repository()
            .with_dot_ignore(false)
            .with_custom_ignore(false)
            .with_git_exclude(false)
            .with_git_global(false);
        let selected = Scanner::new(&fixture.root)
            .options(options)
            .scan()
            .unwrap()
            .files
            .into_iter()
            .map(|file| file.relative)
            .collect::<BTreeSet<_>>();
        let ignored_by_scanner = PATHS
            .iter()
            .filter(|path| !selected.contains(**path))
            .map(|path| (*path).to_owned())
            .collect::<BTreeSet<_>>();
        let ignored_by_git = git_ignored(&fixture.root, PATHS)
            .expect("git check-ignore must accept generated fixture");

        assert_eq!(
            ignored_by_scanner, ignored_by_git,
            "git differential seed {seed}"
        );
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

fn git_init(root: &std::path::Path) -> bool {
    Command::new("git")
        .args(["init", "--quiet"])
        .current_dir(root)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", root.join("missing-global-config"))
        .status()
        .is_ok_and(|status| status.success())
}

fn git_ignored(root: &std::path::Path, paths: &[&str]) -> Option<BTreeSet<String>> {
    let mut child = Command::new("git")
        .args(["check-ignore", "--no-index", "--stdin", "-z"])
        .current_dir(root)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", root.join("missing-global-config"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let mut stdin = child.stdin.take()?;
    for path in paths {
        stdin.write_all(path.as_bytes()).ok()?;
        stdin.write_all(&[0]).ok()?;
    }
    drop(stdin);
    let output = child.wait_with_output().ok()?;
    if !output.status.success() && output.status.code() != Some(1) {
        return None;
    }
    Some(
        output
            .stdout
            .split(|byte| *byte == 0)
            .filter(|path| !path.is_empty())
            .map(|path| String::from_utf8_lossy(path).replace('\\', "/"))
            .collect(),
    )
}
