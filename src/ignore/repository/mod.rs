use super::git::{gitconfig_excludes_path, resolve_git_directory};
use super::overrides::OverrideRules;
use super::repository_source::find_repository_root;
use super::{
    IgnoreRules, RepositoryMatch, RuleAction, SourceRank, match_prepared_rules, match_rules,
    normalized_evidence_location,
};
use crate::config::{IgnorePolicy, ScanOptions};
use crate::error::{Error, Result};
use crate::hidden::is_hidden_with_hint;
use crate::path::normalized_relative_path;
use crate::report::{IgnoreSourceEvidence, IgnoreSourceKind, ScanWarning};
use crate::walk_types::ErrorPolicy;
use std::borrow::Cow;
use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};

mod configuration;
mod matching;

/// A reusable, lazily cached repository ignore matcher.
///
/// Paths may be absolute or relative to the configured scan root. Directory
/// rule files are loaded once and cached, which makes repeated incremental
/// checks cheap without walking the repository again.
#[derive(Debug, Clone)]
pub struct RepositoryMatcher {
    pub(super) scan_root: PathBuf,
    pub(super) options: ScanOptions,
    pub(super) match_root: PathBuf,
    pub(super) scan_base: String,
    pub(super) ignore_files: Vec<String>,
    pub(super) case_insensitive: bool,
    pub(super) error_policy: ErrorPolicy,
    pub(super) overrides: OverrideRules,
    pub(super) skip_hidden: bool,
    pub(super) base_rules: IgnoreRules,
    pub(super) directories: HashMap<PathBuf, IgnoreRules>,
    pub(super) sources: Vec<IgnoreSourceEvidence>,
    pub(super) warnings: Vec<ScanWarning>,
    pub(super) portable: bool,
}

fn source_enabled(name: &str, policy: &IgnorePolicy, git_sources_enabled: bool) -> bool {
    match name {
        ".gitignore" => policy.git_ignore && git_sources_enabled,
        ".ignore" => policy.dot_ignore,
        _ => policy.custom_ignore,
    }
}
