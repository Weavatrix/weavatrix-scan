use super::{IgnoreError, RepositoryMatch, RuleAction, RuleSet, parse_file, source_evidence};
use crate::report::{IgnoreSourceEvidence, IgnoreSourceKind};
use std::path::Path;

#[derive(Debug, Clone, Default)]
pub(super) struct OverrideRules {
    rules: RuleSet,
    has_includes: bool,
}

impl OverrideRules {
    pub(super) fn new(
        patterns: &[String],
        case_insensitive: bool,
    ) -> (Self, Vec<IgnoreError>, Option<IgnoreSourceEvidence>) {
        if patterns.is_empty() {
            return (Self::default(), Vec::new(), None);
        }
        let contents = patterns.join("\n");
        let mut rules = RuleSet::default();
        let mut errors = Vec::new();
        parse_file(
            Path::new("<overrides>"),
            &contents,
            case_insensitive,
            &mut rules,
            &mut errors,
        );
        for rule in &mut rules.rules {
            rule.action = match rule.action {
                RuleAction::Ignore => RuleAction::Include,
                RuleAction::Include => RuleAction::Ignore,
            };
        }
        let has_includes = rules
            .rules
            .iter()
            .any(|rule| rule.action == RuleAction::Include);
        let evidence = source_evidence(
            IgnoreSourceKind::Override,
            "<overrides>".to_owned(),
            contents.as_bytes(),
        );
        (
            Self {
                rules,
                has_includes,
            },
            errors,
            Some(evidence),
        )
    }

    pub(super) fn matched(&self, path: &str, is_directory: bool) -> RepositoryMatch {
        if self.rules.rules.is_empty() {
            return RepositoryMatch::None;
        }
        let matched = self
            .rules
            .matches_exact(path, is_directory)
            .map(|action| match action {
                RuleAction::Ignore => RepositoryMatch::OverrideIgnore,
                RuleAction::Include => RepositoryMatch::OverrideInclude,
            });
        matched.unwrap_or({
            if self.has_includes && !is_directory {
                RepositoryMatch::OverrideIgnore
            } else {
                RepositoryMatch::None
            }
        })
    }
}
