use super::{
    Component, Error, Path, PathBuf, RepositoryMatch, Result, SelectionDecision, SelectionMatcher,
    SkipKind, WalkSkipReason, directory_info, fs, relative_depth, skip_kind, skip_kind_for_match,
};

impl SelectionMatcher {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn classify(
        &mut self,
        absolute: &Path,
        relative: &Path,
        depth: usize,
        is_file: bool,
        is_directory: bool,
        is_symlink: bool,
        bytes: u64,
        walk_skip: Option<WalkSkipReason>,
    ) -> Result<SelectionDecision> {
        if depth == 0 {
            if let Some(reason) = walk_skip {
                return Ok(SelectionDecision::skipped(
                    skip_kind(reason),
                    RepositoryMatch::None,
                ));
            }
            if is_symlink && !self.options.walk.follow_links {
                return Ok(SelectionDecision::skipped(
                    SkipKind::Symlink,
                    RepositoryMatch::None,
                ));
            }
            if is_directory {
                self.repository.prepare_directory(absolute)?;
                return Ok(SelectionDecision::directory(RepositoryMatch::None));
            }
            if is_file {
                return self.classify_file(absolute, relative, bytes);
            }
            return Ok(SelectionDecision::unselected());
        }
        if depth < self.options.effective_min_depth() && !is_directory {
            return Ok(SelectionDecision::unselected());
        }
        if is_symlink && !self.options.walk.follow_links {
            return Ok(SelectionDecision::skipped(
                SkipKind::Symlink,
                RepositoryMatch::None,
            ));
        }
        if let Some(reason) = walk_skip {
            return Ok(SelectionDecision::skipped(
                skip_kind(reason),
                RepositoryMatch::None,
            ));
        }
        if self
            .options
            .walk
            .max_depth
            .is_some_and(|maximum| depth > maximum || (is_directory && depth == maximum))
        {
            return Ok(SelectionDecision::skipped(
                SkipKind::MaxDepth,
                RepositoryMatch::None,
            ));
        }
        if is_directory {
            let matched = self.repository.matched(absolute, true)?;
            if let Some(kind) = skip_kind_for_match(matched) {
                return Ok(SelectionDecision::skipped(kind, matched));
            }
            if matched != RepositoryMatch::OverrideInclude
                && self
                    .options
                    .should_skip_directory(absolute.file_name().unwrap_or(absolute.as_os_str()))
            {
                return Ok(SelectionDecision::skipped(
                    SkipKind::StandardDirectory,
                    matched,
                ));
            }
            self.repository.prepare_directory(absolute)?;
            return Ok(SelectionDecision::directory(matched));
        }
        if is_file {
            return self.classify_file(absolute, relative, bytes);
        }
        Ok(SelectionDecision::unselected())
    }

    fn classify_file(
        &mut self,
        absolute: &Path,
        relative: &Path,
        bytes: u64,
    ) -> Result<SelectionDecision> {
        let matched = self.repository.matched(absolute, false)?;
        if let Some(kind) = skip_kind_for_match(matched) {
            return Ok(SelectionDecision::skipped(kind, matched));
        }
        let normalized = crate::path::normalized_relative_path(relative);
        if matched != RepositoryMatch::OverrideInclude
            && !self.options.accepts_extension(absolute, &normalized)
        {
            return Ok(SelectionDecision::skipped(SkipKind::Extension, matched));
        }
        if bytes > self.options.max_file_bytes {
            return Ok(SelectionDecision::skipped(SkipKind::Oversized, matched));
        }
        Ok(SelectionDecision::selected(matched))
    }

    pub(super) fn match_ancestors(&mut self, relative: &Path) -> Result<Option<SelectionDecision>> {
        let Some(parent) = relative.parent() else {
            return Ok(None);
        };
        let mut current = PathBuf::new();
        for component in parent.components() {
            let Component::Normal(name) = component else {
                continue;
            };
            current.push(name);
            let depth = relative_depth(&current);
            if self
                .options
                .walk
                .max_depth
                .is_some_and(|maximum| depth >= maximum)
            {
                return Ok(Some(SelectionDecision::skipped(
                    SkipKind::MaxDepth,
                    RepositoryMatch::None,
                )));
            }
            let absolute = self.repository.root().join(&current);
            let link_metadata =
                fs::symlink_metadata(&absolute).map_err(|source| Error::io(&absolute, source))?;
            let is_symlink = link_metadata.file_type().is_symlink();
            if is_symlink && !self.options.walk.follow_links {
                return Ok(Some(SelectionDecision::skipped(
                    SkipKind::Symlink,
                    RepositoryMatch::None,
                )));
            }
            let metadata = if is_symlink {
                let canonical = absolute
                    .canonicalize()
                    .map_err(|source| Error::io(&absolute, source))?;
                if !canonical.starts_with(self.repository.root()) {
                    return Ok(Some(SelectionDecision::skipped(
                        SkipKind::PathEscape,
                        RepositoryMatch::None,
                    )));
                }
                fs::metadata(&absolute).map_err(|source| Error::io(&absolute, source))?
            } else {
                link_metadata
            };
            if !metadata.is_dir() {
                return Ok(Some(SelectionDecision::unselected()));
            }
            if let Some(decision) = self.file_system_decision(&absolute, &metadata)? {
                return Ok(Some(decision));
            }
            let matched = self.repository.matched(&absolute, true)?;
            if let Some(kind) = skip_kind_for_match(matched) {
                return Ok(Some(SelectionDecision::skipped(kind, matched)));
            }
            if matched != RepositoryMatch::OverrideInclude
                && self.options.should_skip_directory(name)
            {
                return Ok(Some(SelectionDecision::skipped(
                    SkipKind::StandardDirectory,
                    matched,
                )));
            }
            self.repository.prepare_directory(&absolute)?;
        }
        Ok(None)
    }

    pub(super) fn file_system_decision(
        &self,
        absolute: &Path,
        metadata: &fs::Metadata,
    ) -> Result<Option<SelectionDecision>> {
        let Some(root_file_system) = self.root_file_system else {
            return Ok(None);
        };
        let current = directory_info(absolute, metadata)
            .map_err(|source| Error::io(absolute, source))?
            .file_system;
        Ok((current != root_file_system).then(|| {
            SelectionDecision::skipped(SkipKind::FileSystemBoundary, RepositoryMatch::None)
        }))
    }
}
