use super::repository::RepositoryMatcher;
use super::{
    IgnoreError, IgnoreLayer, IgnoreRules, RuleSet, SourceRank, build_child_rules,
    normalized_evidence_location, source_evidence,
};
use crate::error::{Error, Result};
use crate::path::normalized_relative_path;
use crate::report::{IgnoreSourceEvidence, IgnoreSourceKind, ScanWarning};
use crate::walker::WalkEntry;
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

impl RepositoryMatcher {
    pub(super) fn prepare_directory_inner(&mut self, absolute: &Path) -> Result<()> {
        let ignore_files = self.ignore_files.clone();
        self.prepare_directory_inner_with_files(absolute, &ignore_files)
    }

    pub(crate) fn prepare_directory_from_entries(
        &mut self,
        absolute: &Path,
        entries: &[WalkEntry],
    ) -> Result<()> {
        let ignore_files = self
            .ignore_files
            .iter()
            .filter(|name| {
                entries
                    .iter()
                    .any(|entry| entry.file_name() == name.as_str())
            })
            .cloned()
            .collect::<Vec<_>>();
        self.prepare_directory_inner_with_files(absolute, &ignore_files)
    }

    fn prepare_directory_inner_with_files(
        &mut self,
        absolute: &Path,
        ignore_files: &[String],
    ) -> Result<()> {
        if self.directories.contains_key(absolute) {
            return Ok(());
        }
        let relative = absolute.strip_prefix(&self.match_root).map_err(|_| {
            Error::io(
                absolute,
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "directory escapes matcher root",
                ),
            )
        })?;
        let inherited = if relative.as_os_str().is_empty() {
            self.base_rules.clone()
        } else {
            let parent = absolute.parent().unwrap_or(&self.match_root).to_path_buf();
            self.prepare_directory_inner(&parent)?;
            self.directories
                .get(&parent)
                .cloned()
                .unwrap_or_else(|| self.base_rules.clone())
        };
        let base = normalized_relative_path(relative);
        let (rules, errors, evidence) = build_child_rules(
            absolute,
            &base,
            ignore_files,
            self.case_insensitive,
            &inherited,
            &self.match_root,
        );
        self.handle_errors(errors)?;
        self.sources.extend(evidence);
        self.directories.insert(absolute.to_path_buf(), rules);
        Ok(())
    }

    pub(super) fn absolute_path(&self, path: &Path) -> Result<PathBuf> {
        let escapes = path
            .components()
            .any(|component| component == Component::ParentDir);
        if escapes || (!path.is_absolute() && path.components().any(is_root_component)) {
            return Err(Error::io(
                path,
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "path contains traversal components",
                ),
            ));
        }
        Ok(if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.scan_root.join(path)
        })
    }

    pub(super) fn ensure_scan_scope(&self, path: &Path) -> Result<()> {
        if path.starts_with(&self.scan_root) {
            return Ok(());
        }
        Err(Error::io(
            path,
            io::Error::new(io::ErrorKind::InvalidInput, "path escapes scan root"),
        ))
    }

    pub(super) fn load_static_source(
        &mut self,
        path: &Path,
        base: &str,
        rank: SourceRank,
        kind: IgnoreSourceKind,
        location: &str,
    ) -> Result<()> {
        let (rules, errors, evidence) = add_rule_file(
            &self.base_rules,
            path,
            base,
            rank,
            kind,
            location,
            self.case_insensitive,
        );
        self.handle_errors(errors)?;
        self.base_rules = rules;
        self.sources.extend(evidence);
        Ok(())
    }

    pub(super) fn handle_errors(&mut self, errors: Vec<IgnoreError>) -> Result<()> {
        for source in errors {
            if self.error_policy == crate::walker::ErrorPolicy::Abort {
                return Err(Error::io(
                    &source.path,
                    io::Error::new(source.kind(), source.to_string()),
                ));
            }
            self.warnings.push(ScanWarning {
                relative: Some(normalized_evidence_location(&source.path, &self.match_root)),
                message: format!("could not load ignore rules: {source}"),
            });
        }
        Ok(())
    }
}

pub(super) fn find_repository_root(start: &Path) -> Option<PathBuf> {
    start
        .ancestors()
        .find(|candidate| candidate.join(".git").exists())
        .map(Path::to_path_buf)
}

pub(super) fn add_rule_file(
    inherited: &IgnoreRules,
    path: &Path,
    base: &str,
    rank: SourceRank,
    kind: IgnoreSourceKind,
    location: &str,
    case_insensitive: bool,
) -> (IgnoreRules, Vec<IgnoreError>, Vec<IgnoreSourceEvidence>) {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return (inherited.clone(), Vec::new(), Vec::new());
        }
        Err(error) => {
            return (
                inherited.clone(),
                vec![IgnoreError {
                    kind: error.kind(),
                    path: path.to_path_buf(),
                    message: error.to_string(),
                }],
                Vec::new(),
            );
        }
    };
    let mut rules = RuleSet::default();
    let mut errors = Vec::new();
    super::parser::parse_file_bytes(path, &bytes, case_insensitive, &mut rules, &mut errors);
    let mut result = inherited.clone();
    if !rules.rules.is_empty() {
        let index = rank.index();
        result.layers[index] = Some(Arc::new(IgnoreLayer {
            base: base.to_owned(),
            rules,
            parent: result.layers[index].clone(),
        }));
    }
    (
        result,
        errors,
        vec![source_evidence(kind, location.to_owned(), &bytes)],
    )
}

fn is_root_component(component: Component<'_>) -> bool {
    matches!(component, Component::RootDir | Component::Prefix(_))
}
