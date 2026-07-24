use crate::hash::FingerprintHasher;
use crate::path::normalized_relative_path;
use crate::report::{IgnoreSourceEvidence, IgnoreSourceKind};
use std::collections::HashMap;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

mod git;
mod matcher;
mod overrides;
mod parser;
mod repository;
mod repository_source;
mod rules;
#[cfg(test)]
mod tests;

#[cfg(test)]
use git::{expand_home, read_excludes_setting, read_excludes_setting_for, resolve_git_directory};
use matcher::RuleMatcher;
use parser::parse_file;
pub use repository::RepositoryMatcher;
#[cfg(test)]
use repository_source::{add_rule_file, find_repository_root};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IgnoreFile {
    pub name: String,
}

/// Highest-precedence selection decision for a repository path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepositoryMatch {
    None,
    Ignore,
    Include,
    OverrideIgnore,
    OverrideInclude,
    Hidden,
}

impl RepositoryMatch {
    #[must_use]
    pub const fn is_ignored(self) -> bool {
        matches!(self, Self::Ignore | Self::OverrideIgnore | Self::Hidden)
    }
}

const SOURCE_COUNT: usize = 6;

#[derive(Debug, Clone, Default)]
pub(crate) struct IgnoreRules {
    layers: [Option<Arc<IgnoreLayer>>; SOURCE_COUNT],
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

#[derive(Debug, Clone, Copy)]
enum RuleMatch {
    Exact(RuleAction),
    Ancestor(RuleAction),
}

#[derive(Debug, Clone, Copy)]
enum SourceRank {
    GitGlobal = 0,
    GitExclude = 1,
    GitIgnore = 2,
    DotIgnore = 3,
    Custom = 4,
    Explicit = 5,
}

impl SourceRank {
    const fn index(self) -> usize {
        self as usize
    }
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
    evidence_root: &Path,
) -> (IgnoreRules, Vec<IgnoreError>, Vec<IgnoreSourceEvidence>) {
    let mut result = inherited.clone();
    let mut rules_by_source: [RuleSet; SOURCE_COUNT] = Default::default();
    let mut errors = Vec::new();
    let mut evidence = Vec::new();
    for name in ignore_files {
        let path = directory.join(name);
        let bytes = match read_local_rule_file(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => {
                errors.push(IgnoreError {
                    kind: error.kind(),
                    path,
                    message: error.to_string(),
                });
                continue;
            }
        };
        let (rank, kind) = source_for_name(name);
        parser::parse_file_bytes(
            &path,
            &bytes,
            case_insensitive,
            &mut rules_by_source[rank.index()],
            &mut errors,
        );
        evidence.push(source_evidence(
            kind,
            normalized_evidence_location(&path, evidence_root),
            &bytes,
        ));
    }
    for (index, rules) in rules_by_source.into_iter().enumerate() {
        if rules.rules.is_empty() {
            continue;
        }
        result.layers[index] = Some(Arc::new(IgnoreLayer {
            base: base.to_owned(),
            rules,
            parent: result.layers[index].clone(),
        }));
    }
    (result, errors, evidence)
}

fn match_rules(path: &str, is_directory: bool, rules: &IgnoreRules) -> Option<RuleAction> {
    let mut ancestor_included = false;
    for source in rules.layers.iter().rev() {
        let mut layer = source.as_deref();
        while let Some(current) = layer {
            if let Some(candidate) = candidate_for_base(path, &current.base)
                && let Some(rule_match) = current.rules.matches(candidate, is_directory)
            {
                match rule_match {
                    RuleMatch::Exact(action) => return Some(action),
                    RuleMatch::Ancestor(RuleAction::Include) => ancestor_included = true,
                    RuleMatch::Ancestor(RuleAction::Ignore) if !ancestor_included => {
                        return Some(RuleAction::Ignore);
                    }
                    RuleMatch::Ancestor(RuleAction::Ignore) => {}
                }
            }
            layer = current.parent.as_deref();
        }
    }
    ancestor_included.then_some(RuleAction::Include)
}

fn candidate_for_base<'a>(path: &'a str, base: &str) -> Option<&'a str> {
    if base.is_empty() {
        Some(path)
    } else {
        path.strip_prefix(base)?.strip_prefix('/')
    }
}

fn source_for_name(name: &str) -> (SourceRank, IgnoreSourceKind) {
    match name {
        ".gitignore" => (SourceRank::GitIgnore, IgnoreSourceKind::GitIgnore),
        ".ignore" => (SourceRank::DotIgnore, IgnoreSourceKind::DotIgnore),
        _ => (SourceRank::Custom, IgnoreSourceKind::Custom),
    }
}

fn source_evidence(
    kind: IgnoreSourceKind,
    location: String,
    contents: &[u8],
) -> IgnoreSourceEvidence {
    let mut hash = FingerprintHasher::new();
    hash.write(contents);
    IgnoreSourceEvidence {
        kind,
        location,
        content_hash: hash.finish(),
    }
}

fn normalized_evidence_location(path: &Path, root: &Path) -> String {
    path.strip_prefix(root).map_or_else(
        |_| path.to_string_lossy().replace('\\', "/"),
        normalized_relative_path,
    )
}

fn read_local_rule_file(path: &Path) -> io::Result<Vec<u8>> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "ignore file is a symbolic link",
        ));
    }
    fs::read(path)
}
