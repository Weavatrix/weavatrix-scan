use crate::report::{ScanReport, SkipKind};
use std::collections::HashMap;
use std::path::Path;

mod matcher;

use matcher::RuleMatcher;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IgnoreFile {
    pub name: String,
}

#[derive(Debug, Clone)]
pub(crate) struct IgnoreRule {
    base: String,
    pattern: String,
    action: RuleAction,
    target: RuleTarget,
    scope: RuleScope,
    matcher: RuleMatcher,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct IgnoreRules {
    rules: Vec<IgnoreRule>,
    exact_anywhere: HashMap<String, Vec<usize>>,
    complex: Vec<usize>,
}

impl IgnoreRules {
    fn push(&mut self, rule: IgnoreRule) {
        let index = self.rules.len();
        if rule.scope == RuleScope::Anywhere && rule.matcher.is_literal() {
            self.exact_anywhere
                .entry(rule.pattern.clone())
                .or_default()
                .push(index);
        } else {
            self.complex.push(index);
        }
        self.rules.push(rule);
    }
}

impl FromIterator<IgnoreRule> for IgnoreRules {
    fn from_iter<T: IntoIterator<Item = IgnoreRule>>(iter: T) -> Self {
        let mut rules = Self::default();
        for rule in iter {
            rules.push(rule);
        }
        rules
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RuleAction {
    Ignore,
    Include,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RuleTarget {
    Any,
    Directory,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RuleScope {
    Anywhere,
    Path,
    Anchored,
}

impl IgnoreRule {
    fn matches(&self, repository_path: &str, is_directory: bool) -> bool {
        let Some(candidate) = candidate_for_base(repository_path, &self.base) else {
            return false;
        };

        if self.scope != RuleScope::Anywhere {
            return path_or_ancestor_matches(self, candidate, is_directory, self.target);
        }

        let mut components = candidate.split('/').peekable();
        while let Some(component) = components.next() {
            let is_ancestor = components.peek().is_some();
            if (self.target != RuleTarget::Directory || is_directory || is_ancestor)
                && self.matches_value(component)
            {
                return true;
            }
        }
        false
    }

    fn matches_value(&self, value: &str) -> bool {
        self.matcher.matches(&self.pattern, value)
    }
}

pub(crate) fn load_ignore_file(
    path: &Path,
    base: &str,
    rules: &mut IgnoreRules,
    report: &mut ScanReport,
) {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
        Err(error) => {
            report.warn(
                Some(base.to_owned()),
                format!("could not read ignore file: {error}"),
            );
            return;
        }
    };
    for raw in text.lines() {
        if let Some(rule) = parse_rule(raw, base) {
            rules.push(rule);
        }
    }
}

pub(crate) fn is_ignored(path: &str, is_directory: bool, rules: &IgnoreRules) -> bool {
    let mut best = None;
    for component in path.split('/') {
        let Some(indices) = rules.exact_anywhere.get(component) else {
            continue;
        };
        if let Some(&index) = indices
            .iter()
            .rev()
            .find(|&&index| rules.rules[index].matches(path, is_directory))
        {
            best = Some(best.map_or(index, |current: usize| current.max(index)));
        }
    }
    for &index in rules.complex.iter().rev() {
        if best.is_some_and(|best| index <= best) {
            break;
        }
        if rules.rules[index].matches(path, is_directory) {
            return rules.rules[index].action == RuleAction::Ignore;
        }
    }
    best.is_some_and(|index| rules.rules[index].action == RuleAction::Ignore)
}

pub(crate) fn skip_ignored(
    report: &mut ScanReport,
    relative: &str,
    is_directory: bool,
    rules: &IgnoreRules,
) -> bool {
    let ignored = is_ignored(relative, is_directory, rules);
    if ignored {
        report.skip(relative.to_owned(), SkipKind::Ignored, None);
    }
    ignored
}

fn parse_rule(raw: &str, base: &str) -> Option<IgnoreRule> {
    let mut line = trim_unescaped_trailing_spaces(raw.trim_end_matches('\r')).to_owned();
    if line.is_empty() {
        return None;
    }
    let escaped_prefix = line.starts_with("\\#") || line.starts_with("\\!");
    if escaped_prefix {
        line.remove(0);
    } else if line.starts_with('#') {
        return None;
    }
    let negated = !escaped_prefix && line.starts_with('!');
    if negated {
        line.remove(0);
    }
    let target = if line.ends_with('/') {
        line.pop();
        RuleTarget::Directory
    } else {
        RuleTarget::Any
    };
    let anchored = line.starts_with('/');
    if anchored {
        line.remove(0);
    }
    if line.is_empty() {
        return None;
    }
    let scope = if anchored {
        RuleScope::Anchored
    } else if line.contains('/') {
        RuleScope::Path
    } else {
        RuleScope::Anywhere
    };
    Some(IgnoreRule {
        base: base.to_owned(),
        matcher: RuleMatcher::new(&line),
        pattern: line,
        action: if negated {
            RuleAction::Include
        } else {
            RuleAction::Ignore
        },
        target,
        scope,
    })
}

fn candidate_for_base<'a>(repository_path: &'a str, base: &str) -> Option<&'a str> {
    if base.is_empty() {
        Some(repository_path)
    } else if repository_path == base {
        Some("")
    } else {
        repository_path
            .strip_prefix(base)
            .and_then(|candidate| candidate.strip_prefix('/'))
    }
}

fn path_or_ancestor_matches(
    rule: &IgnoreRule,
    candidate: &str,
    is_directory: bool,
    target: RuleTarget,
) -> bool {
    if rule.matches_value(candidate) && (target != RuleTarget::Directory || is_directory) {
        return true;
    }
    candidate
        .match_indices('/')
        .any(|(separator, _)| rule.matches_value(&candidate[..separator]))
}

fn trim_unescaped_trailing_spaces(mut line: &str) -> &str {
    while let Some(without_space) = line.strip_suffix(' ') {
        let slash_count = without_space
            .bytes()
            .rev()
            .take_while(|byte| *byte == b'\\')
            .count();
        if slash_count % 2 == 1 {
            break;
        }
        line = without_space;
    }
    line
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordered_negation_reincludes_a_file() {
        let rules = [
            parse_rule("*.rs", "").unwrap(),
            parse_rule("!lib.rs", "").unwrap(),
        ]
        .into_iter()
        .collect();
        assert!(!is_ignored("src/lib.rs", false, &rules));
        assert!(is_ignored("src/generated.rs", false, &rules));
    }

    #[test]
    fn preserves_escaped_trailing_spaces() {
        let rule = parse_rule(r"name\ ", "").unwrap();
        assert!(rule.matches("name ", false));
        assert!(!rule.matches("name", false));
    }
}
