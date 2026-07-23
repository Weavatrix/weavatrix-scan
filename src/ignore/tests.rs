use super::parser::parse_rule;
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
fn valid_gitignore_patterns_match_the_reference_backend() {
    use ignore::gitignore::GitignoreBuilder;

    const PATTERNS: &[&str] = &[
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
    const CANDIDATES: &[(&str, bool)] = &[
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

    for pattern in PATTERNS {
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
        for &(candidate, is_directory) in CANDIDATES {
            let ours = ours.matches(candidate, is_directory);
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
