use crate::report::{ScanReport, SkipKind};
use std::collections::HashMap;
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

mod matcher;
mod parser;
#[cfg(test)]
mod tests;

use matcher::RuleMatcher;
use parser::parse_file;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IgnoreFile {
    pub name: String,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct IgnoreRules {
    layer: Option<Arc<IgnoreLayer>>,
}

#[derive(Debug)]
struct IgnoreLayer {
    base: String,
    rules: RuleSet,
    parent: Option<Arc<IgnoreLayer>>,
}

#[derive(Debug, Default)]
struct RuleSet {
    rules: Vec<IgnoreRule>,
    exact_anywhere: HashMap<String, Vec<usize>>,
    prefixes: HashMap<u8, Vec<usize>>,
    suffixes: HashMap<u8, Vec<usize>>,
    generic: Vec<usize>,
}

#[derive(Debug)]
struct IgnoreRule {
    pattern: String,
    action: RuleAction,
    target: RuleTarget,
    scope: RuleScope,
    matcher: RuleMatcher,
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

#[derive(Debug)]
pub(crate) struct IgnoreError {
    kind: io::ErrorKind,
    path: PathBuf,
    message: String,
}

impl IgnoreError {
    pub(crate) const fn kind(&self) -> io::ErrorKind {
        self.kind
    }
}

impl fmt::Display for IgnoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.path.display(), self.message)
    }
}

impl std::error::Error for IgnoreError {}

pub(crate) fn build_child_rules(
    directory: &Path,
    base: &str,
    ignore_files: &[String],
    case_insensitive: bool,
    inherited: &IgnoreRules,
) -> (IgnoreRules, Vec<IgnoreError>) {
    let mut rules = RuleSet::default();
    let mut errors = Vec::new();
    let mut found = false;
    for name in ignore_files {
        let path = directory.join(name);
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => {
                found = true;
                text
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => {
                found = true;
                errors.push(IgnoreError {
                    kind: error.kind(),
                    path,
                    message: error.to_string(),
                });
                continue;
            }
        };
        parse_file(&path, &text, case_insensitive, &mut rules, &mut errors);
    }
    if !found || rules.rules.is_empty() {
        return (inherited.clone(), errors);
    }
    (
        IgnoreRules {
            layer: Some(Arc::new(IgnoreLayer {
                base: base.to_owned(),
                rules,
                parent: inherited.layer.clone(),
            })),
        },
        errors,
    )
}

pub(crate) fn is_ignored(path: &str, is_directory: bool, rules: &IgnoreRules) -> bool {
    let mut layer = rules.layer.as_deref();
    while let Some(current) = layer {
        if let Some(candidate) = candidate_for_base(path, &current.base)
            && let Some(action) = current.rules.matches(candidate, is_directory)
        {
            return action == RuleAction::Ignore;
        }
        layer = current.parent.as_deref();
    }
    false
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

impl RuleSet {
    fn push(&mut self, rule: IgnoreRule) {
        let index = self.rules.len();
        if rule.scope == RuleScope::Anywhere && rule.matcher.is_literal() {
            self.exact_anywhere
                .entry(rule.pattern.clone())
                .or_default()
                .push(index);
        } else if let Some(key) = rule.matcher.prefix_key() {
            self.prefixes.entry(key).or_default().push(index);
        } else if let Some(key) = rule.matcher.suffix_key(&rule.pattern) {
            self.suffixes.entry(key).or_default().push(index);
        } else {
            self.generic.push(index);
        }
        self.rules.push(rule);
    }

    fn matches(&self, path: &str, is_directory: bool) -> Option<RuleAction> {
        if let Some(action) = self.matches_exact(path, is_directory) {
            return Some(action);
        }
        let mut ancestor = path;
        while let Some((parent, _)) = ancestor.rsplit_once('/') {
            if let Some(action) = self.matches_exact(parent, true) {
                return Some(action);
            }
            ancestor = parent;
        }
        None
    }

    fn matches_exact(&self, path: &str, is_directory: bool) -> Option<RuleAction> {
        let mut best = None;
        let name = path.rsplit('/').next().unwrap_or(path);
        if let Some(indices) = self.exact_anywhere.get(name)
            && let Some(&index) = indices
                .iter()
                .rev()
                .find(|&&index| self.rules[index].matches_exact(path, is_directory))
        {
            best = Some(index);
        }
        best = self.best_match(&self.generic, path, is_directory, best);
        let name_prefix = name.as_bytes().first();
        if let Some(indices) = name_prefix.and_then(|key| self.prefixes.get(key)) {
            best = self.best_match(indices, path, is_directory, best);
        }
        if let Some(path_prefix) = path.as_bytes().first()
            && Some(path_prefix) != name_prefix
            && let Some(indices) = self.prefixes.get(path_prefix)
        {
            best = self.best_match(indices, path, is_directory, best);
        }
        if let Some(indices) = path
            .as_bytes()
            .last()
            .and_then(|key| self.suffixes.get(key))
        {
            best = self.best_match(indices, path, is_directory, best);
        }
        best.map(|index| self.rules[index].action)
    }

    fn best_match(
        &self,
        indices: &[usize],
        path: &str,
        is_directory: bool,
        mut best: Option<usize>,
    ) -> Option<usize> {
        for &index in indices.iter().rev() {
            if best.is_some_and(|best| index <= best) {
                break;
            }
            if self.rules[index].matches_exact(path, is_directory) {
                best = Some(index);
                break;
            }
        }
        best
    }
}

impl IgnoreRule {
    fn matches_exact(&self, path: &str, is_directory: bool) -> bool {
        if self.target == RuleTarget::Directory && !is_directory {
            return false;
        }
        if self.scope == RuleScope::Anywhere {
            let name = path.rsplit('/').next().unwrap_or(path);
            return self.matcher.matches(&self.pattern, name);
        }
        self.matcher.matches(&self.pattern, path)
    }
}

fn candidate_for_base<'a>(path: &'a str, base: &str) -> Option<&'a str> {
    if base.is_empty() {
        Some(path)
    } else {
        path.strip_prefix(base)?.strip_prefix('/')
    }
}
