use super::parser::{parse_file_bytes, parse_rule};
use super::*;

#[test]
fn parser_handles_negation_case_and_invalid_patterns() {
    let rule = parse_rule("!DROP.RS", true).unwrap().unwrap();
    assert_eq!(rule.action, RuleAction::Include);
    assert!(rule.matcher.matches(&rule.pattern, "drop.rs"));
    assert!(parse_rule("{broken", false).is_err());
}

#[test]
fn preserves_escaped_trailing_spaces() {
    let rule = parse_rule(r"name\ ", false).unwrap().unwrap();
    assert!(rule.matcher.matches(&rule.pattern, "name "));
    assert!(!rule.matcher.matches(&rule.pattern, "name"));
}

#[test]
fn arbitrary_gitignore_grammar_is_panic_free_and_deterministic() {
    let cases = std::env::var("WEAVATRIX_FUZZ_CASES")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(4_096)
        .min(1_000_000);
    for seed in 0..cases {
        let mut state = seed.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let length = usize::try_from(next_fuzz(&mut state) % 384).unwrap();
        let mut input = Vec::with_capacity(length);
        for _ in 0..length {
            input.push(next_fuzz(&mut state).to_le_bytes()[0]);
        }
        let mut first = RuleSet::default();
        let mut first_errors = Vec::new();
        parse_file_bytes(
            Path::new("fuzz/.gitignore"),
            &input,
            seed % 2 == 0,
            &mut first,
            &mut first_errors,
        );
        let mut second = RuleSet::default();
        let mut second_errors = Vec::new();
        parse_file_bytes(
            Path::new("fuzz/.gitignore"),
            &input,
            seed % 2 == 0,
            &mut second,
            &mut second_errors,
        );
        assert_eq!(first.rules.len(), second.rules.len(), "seed {seed}");
        assert_eq!(first_errors.len(), second_errors.len(), "seed {seed}");

        let candidate = format!(
            "nested/{:016x}/candidate-{}.rs",
            next_fuzz(&mut state),
            seed % 17
        );
        assert_eq!(
            first.matches(&candidate, seed % 3 == 0).map(rule_action),
            second.matches(&candidate, seed % 3 == 0).map(rule_action),
            "seed {seed}"
        );
    }
}

const fn next_fuzz(state: &mut u64) -> u64 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    *state
}

const fn rule_action(matched: RuleMatch) -> RuleAction {
    match matched {
        RuleMatch::Exact(action) | RuleMatch::Ancestor(action) => action,
    }
}

const VALID_PATTERNS: &[&str] = &[
    "*.rs",
    "!keep.rs",
    "/root.rs",
    "docs/",
    "foo/bar",
    "foo/**/bar",
    "abc/**",
    "**/cache/*.tmp",
    "[ab].rs",
    "[!a-c].rs",
    r"file\[1\].rs",
    r"\#literal",
    r"\!literal",
    r"name\ space",
    r"name\ ",
    r"foo\/",
    "{foo,bar}.rs",
    "a/***/b",
    "[literal",
    "foo*",
    "*foo",
    "src/*",
    "/foo*",
    "foo?",
    "foo[0-9]",
    "foo[!0-9].txt",
    "**/foo",
    "foo/**",
    "foo/**/*",
    "a/**/b/**/c",
    "a*b/c",
    "/foo/bar",
    "foo/bar/",
    "!foo/bar",
    "**",
];

const VALID_CANDIDATES: &[(&str, bool)] = &[
    ("root.rs", false),
    ("src/root.rs", false),
    ("keep.rs", false),
    ("src/keep.rs", false),
    ("drop.rs", false),
    ("docs", true),
    ("docs/guide.rs", false),
    ("src/docs", true),
    ("foo", true),
    ("foo/bar", false),
    ("foo/a/bar", false),
    ("foo/a/b/bar", false),
    ("abc", true),
    ("abc/child", false),
    ("src/cache/drop.tmp", false),
    ("cache/keep.rs", false),
    ("a.rs", false),
    ("z.rs", false),
    ("file[1].rs", false),
    ("#literal", false),
    ("!literal", false),
    ("name space", false),
    ("name ", false),
    ("foo.rs", false),
    ("bar.rs", false),
    ("a/x/b", false),
    ("[literal", false),
    ("foo0", false),
    ("foo9", false),
    ("fooa", false),
    ("foo7.txt", false),
    ("fooa.txt", false),
    ("nested/foo", false),
    ("foo/child", false),
    ("foo/deep/child", false),
    ("a/x/b/y/c", false),
    ("axxb/c", false),
    ("nested/foo/bar", false),
    ("foo/bar", true),
];

#[test]
fn valid_gitignore_patterns_match_the_reference_backend() {
    use ignore::gitignore::GitignoreBuilder;

    for pattern in VALID_PATTERNS {
        let mut ours = RuleSet::default();
        let rule = parse_rule(pattern, false)
            .unwrap_or_else(|error| panic!("{pattern:?}: {error}"))
            .expect("test patterns are not comments");
        ours.push(rule);

        let mut builder = GitignoreBuilder::new("");
        builder
            .add_line(None, pattern)
            .unwrap_or_else(|error| panic!("{pattern:?}: {error}"));
        let reference = builder.build().unwrap();
        for &(candidate, is_directory) in VALID_CANDIDATES {
            let ours = ours
                .matches(candidate, is_directory)
                .map(|matched| match matched {
                    RuleMatch::Exact(action) | RuleMatch::Ancestor(action) => action,
                });
            let matched = reference.matched_path_or_any_parents(candidate, is_directory);
            let expected = if matched.is_ignore() {
                Some(RuleAction::Ignore)
            } else if matched.is_whitelist() {
                Some(RuleAction::Include)
            } else {
                None
            };
            assert_eq!(
                ours, expected,
                "pattern={pattern:?} candidate={candidate:?} dir={is_directory}"
            );
        }
    }
}

#[test]
fn parses_git_core_excludes_file_and_worktree_gitdir() {
    let root = std::env::temp_dir().join(format!("weavatrix-ignore-config-{}", std::process::id()));
    let worktree = root.join("worktree");
    let git_directory = root.join("metadata");
    std::fs::create_dir_all(&worktree).unwrap();
    std::fs::create_dir_all(&git_directory).unwrap();
    std::fs::write(
        root.join("gitconfig"),
        "[core]\n  excludesFile = \"~/.config/git/custom-ignore\"\n",
    )
    .unwrap();
    std::fs::write(worktree.join(".git"), "gitdir: ../metadata\n").unwrap();

    let configured = read_excludes_setting(&root.join("gitconfig")).unwrap();
    assert!(configured.ends_with(".config/git/custom-ignore"));
    assert_eq!(
        resolve_git_directory(&worktree).unwrap(),
        worktree.join("../metadata")
    );

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn repository_and_source_helpers_cover_portable_edge_cases() {
    let root =
        std::env::temp_dir().join(format!("weavatrix-ignore-helpers-{}", std::process::id()));
    let repository = root.join("repository");
    let nested = repository.join("packages/app");
    std::fs::create_dir_all(repository.join(".git/info")).unwrap();
    std::fs::create_dir_all(&nested).unwrap();
    std::fs::write(
        root.join("dotted-config"),
        "core.excludesFile = relative-ignore\n",
    )
    .unwrap();

    assert_eq!(
        find_repository_root(&nested).as_deref(),
        Some(repository.as_path())
    );
    assert_eq!(
        resolve_git_directory(&repository).as_deref(),
        Some(repository.join(".git").as_path())
    );
    assert_eq!(
        read_excludes_setting(&root.join("dotted-config")).as_deref(),
        Some(root.join("relative-ignore").as_path())
    );
    assert_eq!(expand_home("ordinary/path"), PathBuf::from("ordinary/path"));
    assert_eq!(
        normalized_evidence_location(&nested, &repository),
        "packages/app"
    );
    assert!(normalized_evidence_location(&repository, &nested).contains("repository"));
    assert_eq!(
        source_for_name(".gitignore").0.index(),
        SourceRank::GitIgnore.index()
    );
    assert_eq!(source_for_name(".ignore").1, IgnoreSourceKind::DotIgnore);
    assert_eq!(source_for_name(".custom").1, IgnoreSourceKind::Custom);

    let (rules, errors, evidence) = add_rule_file(
        &IgnoreRules::default(),
        &root.join("missing"),
        "",
        SourceRank::Explicit,
        IgnoreSourceKind::Explicit,
        "missing",
        false,
    );
    assert!(rules.layers.iter().all(Option::is_none));
    assert!(errors.is_empty());
    assert!(evidence.is_empty());

    let _ = std::fs::remove_dir_all(root);
}
