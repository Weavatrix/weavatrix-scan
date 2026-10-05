use weavatrix_scan::{PathPattern, PathPatternErrorKind};

#[test]
fn wildcards_preserve_repository_path_boundaries() {
    let file = PathPattern::new("*.rs").unwrap();
    assert!(file.is_match("lib.rs"));
    assert!(!file.is_match("src/lib.rs"));

    let recursive = PathPattern::new("src/**/generated.rs").unwrap();
    assert!(recursive.is_match("src/generated.rs"));
    assert!(recursive.is_match("src/a/b/generated.rs"));

    let one_directory = PathPattern::new("src/*/generated.rs").unwrap();
    assert!(one_directory.is_match("src/a/generated.rs"));
    assert!(!one_directory.is_match("src/a/b/generated.rs"));
}

#[test]
fn alternatives_classes_and_escapes_match_real_paths() {
    let alternatives = PathPattern::new("src/{api,domain}/**/*.rs").unwrap();
    assert_eq!(alternatives.as_str(), "src/{api,domain}/**/*.rs");
    assert!(alternatives.is_match("src/api/lib.rs"));
    assert!(alternatives.is_match("src/domain/model/order.rs"));
    assert!(!alternatives.is_match("src/ui/lib.rs"));

    assert!(
        PathPattern::new("src/file?.rs")
            .unwrap()
            .is_match("src/file1.rs")
    );
    assert!(
        PathPattern::new("src/[a-c].rs")
            .unwrap()
            .is_match("src/b.rs")
    );
    assert!(
        PathPattern::new("src/[!a-c].rs")
            .unwrap()
            .is_match("src/z.rs")
    );
    assert!(
        !PathPattern::new("src/[!a-c].rs")
            .unwrap()
            .is_match("src/b.rs")
    );
    assert!(
        PathPattern::new(r"src/file\[1\].rs")
            .unwrap()
            .is_match("src/file[1].rs")
    );
    assert!(
        PathPattern::new(r"src/\{literal\}.rs")
            .unwrap()
            .is_match("src/{literal}.rs")
    );
}

#[test]
fn malformed_patterns_report_the_failed_syntax_position() {
    let cases = [
        ("", PathPatternErrorKind::Empty, 0),
        (r"src/\", PathPatternErrorKind::DanglingEscape, 4),
        ("src/[abc", PathPatternErrorKind::UnclosedCharacterClass, 4),
        (
            "src/[z-a].rs",
            PathPatternErrorKind::InvalidCharacterRange,
            6,
        ),
        (
            "src/{api,}.rs",
            PathPatternErrorKind::EmptyBraceAlternative,
            9,
        ),
        (
            "src/{,api}.rs",
            PathPatternErrorKind::EmptyBraceAlternative,
            5,
        ),
        ("src/{api.rs", PathPatternErrorKind::UnclosedBrace, 4),
        (
            "src/api}.rs",
            PathPatternErrorKind::UnexpectedClosingBrace,
            7,
        ),
    ];

    for (source, expected_kind, expected_position) in cases {
        let error = PathPattern::new(source).unwrap_err();
        assert_eq!(error.kind(), expected_kind, "pattern {source:?}");
        assert_eq!(error.position(), expected_position, "pattern {source:?}");
        assert_ne!(error.to_string(), "", "pattern {source:?}");
    }
}

#[test]
fn alternative_expansion_is_bounded_before_matching() {
    let source = "{a,b}".repeat(9);
    let error = PathPattern::new(source).unwrap_err();

    assert_eq!(error.kind(), PathPatternErrorKind::ExpansionLimitExceeded);
    assert_eq!(error.position(), 40);
}
