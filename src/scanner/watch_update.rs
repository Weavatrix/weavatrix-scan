use super::entry::record_walk_error;
use super::scan_repository_with_runtime;
use crate::WatchPlan;
use crate::config::ScanOptions;
use crate::content::inspect_files;
use crate::error::{Error, Result};
use crate::file_version::from_metadata;
use crate::ignore::{RepositoryMatch, RepositoryMatcher};
use crate::path::normalized_relative_path;
use crate::report::{ScanCacheStats, ScanReport, ScannedFile, SkipKind};
use crate::runtime::ParallelRuntime;
use crate::scan_finalize::finalize_report;
use crate::scan_limits::apply_total_bytes_limit;
use crate::scan_match::skip_match;
use crate::walk_types::{WalkError, WalkOperation};
use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::{Component, Path};
use std::time::Instant;

pub(super) fn scan_watch_plan(
    root: &Path,
    options: &ScanOptions,
    previous: &ScanReport,
    plan: &WatchPlan,
    parallel_runtime: &ParallelRuntime,
) -> Result<ScanReport> {
    if plan.full_rescan
        || !previous.complete
        || previous.termination.is_some()
        || options.limits.max_entries.is_some()
    {
        return scan_repository_with_runtime(
            root,
            options,
            Some(&previous.to_cache()),
            parallel_runtime,
        );
    }
    let canonical = root
        .canonicalize()
        .map_err(|source| Error::io(root, source))?;
    if canonical != previous.root || !canonical.is_dir() {
        return scan_repository_with_runtime(root, options, None, parallel_runtime);
    }
    if plan
        .invalidated()
        .any(|relative| !is_safe_relative(relative))
    {
        return scan_repository_with_runtime(
            root,
            options,
            Some(&previous.to_cache()),
            parallel_runtime,
        );
    }

    let invalidated = plan.invalidated().collect::<HashSet<_>>();
    let mut report = previous.clone();
    report
        .files
        .retain(|file| !invalidated.contains(file.relative.as_str()));
    report
        .skipped
        .retain(|entry| !invalidated.contains(entry.relative.as_str()));
    report.warnings.retain(|warning| {
        warning
            .relative
            .as_deref()
            .is_none_or(|relative| !invalidated.contains(relative))
    });
    report.revision.clear();
    report.complete = true;
    report.termination = None;
    report.cache = ScanCacheStats::default();

    let mut matcher = RepositoryMatcher::with_options(&canonical, options)?;
    let mut changed_candidates = Vec::new();
    for relative in &plan.changed {
        match changed_candidate(&canonical, relative, options, &mut matcher, &mut report)? {
            ChangedPath::Candidate(file) => changed_candidates.push(*file),
            ChangedPath::MissingOrSkipped => {}
            ChangedPath::NeedsFullScan => {
                return scan_repository_with_runtime(
                    root,
                    options,
                    Some(&previous.to_cache()),
                    parallel_runtime,
                );
            }
        }
    }
    if matcher
        .sources()
        .iter()
        .any(|source| !previous.ignore_sources.contains(source))
    {
        return scan_repository_with_runtime(
            root,
            options,
            Some(&previous.to_cache()),
            parallel_runtime,
        );
    }

    let inspected = inspect_files(changed_candidates, options, Instant::now(), None)?;
    report.files.extend(inspected.files);
    report.skipped.extend(inspected.skipped);
    report.cache = inspected.cache;
    if let Some(reason) = inspected.termination {
        report.terminate(reason);
    }
    for warning in inspected.warnings {
        report.warn(warning.relative, warning.message);
    }
    for warning in matcher.warnings() {
        if !report.warnings.contains(warning) {
            report.warnings.push(warning.clone());
            report.complete = false;
        }
    }
    apply_total_bytes_limit(&mut report, options);
    finalize_report(&mut report);
    Ok(report)
}

enum ChangedPath {
    Candidate(Box<ScannedFile>),
    MissingOrSkipped,
    NeedsFullScan,
}

#[allow(clippy::too_many_lines)]
fn changed_candidate(
    root: &Path,
    relative: &str,
    options: &ScanOptions,
    matcher: &mut RepositoryMatcher,
    report: &mut ScanReport,
) -> Result<ChangedPath> {
    let path = root.join(Path::new(relative));
    let link_metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(source) if source.kind() == io::ErrorKind::NotFound => {
            return Ok(ChangedPath::MissingOrSkipped);
        }
        Err(source) => {
            return local_error(
                &path,
                relative,
                WalkOperation::ReadMetadata,
                source,
                options,
                report,
            );
        }
    };
    let is_symlink = link_metadata.file_type().is_symlink();
    if is_symlink && !options.walk.follow_links {
        report.skip(relative.to_owned(), SkipKind::Symlink, None);
        return Ok(ChangedPath::MissingOrSkipped);
    }
    let metadata = if is_symlink {
        let canonical = match path.canonicalize() {
            Ok(canonical) => canonical,
            Err(source) => {
                return local_error(
                    &path,
                    relative,
                    WalkOperation::Canonicalize,
                    source,
                    options,
                    report,
                );
            }
        };
        if !canonical.starts_with(root) {
            report.skip(relative.to_owned(), SkipKind::PathEscape, None);
            return Ok(ChangedPath::MissingOrSkipped);
        }
        match fs::metadata(&path) {
            Ok(metadata) => metadata,
            Err(source) => {
                return local_error(
                    &path,
                    relative,
                    WalkOperation::ReadMetadata,
                    source,
                    options,
                    report,
                );
            }
        }
    } else {
        link_metadata
    };
    if metadata.is_dir() {
        return Ok(ChangedPath::NeedsFullScan);
    }
    if !metadata.is_file() {
        return Ok(ChangedPath::MissingOrSkipped);
    }

    let depth = Path::new(relative).components().count();
    if options
        .walk
        .max_depth
        .is_some_and(|maximum| depth > maximum)
    {
        report.skip(relative.to_owned(), SkipKind::MaxDepth, None);
        return Ok(ChangedPath::MissingOrSkipped);
    }
    if depth < options.effective_min_depth() {
        return Ok(ChangedPath::MissingOrSkipped);
    }
    if !ancestors_selected(root, relative, options, matcher, report)? {
        return Ok(ChangedPath::MissingOrSkipped);
    }
    let decision = matcher.matched(&path, false)?;
    if skip_match(report, relative.to_owned(), decision) {
        return Ok(ChangedPath::MissingOrSkipped);
    }
    if decision != RepositoryMatch::OverrideInclude && !options.accepts_extension(&path, relative) {
        report.skip(relative.to_owned(), SkipKind::Extension, None);
        return Ok(ChangedPath::MissingOrSkipped);
    }
    let bytes = metadata.len();
    if bytes > options.max_file_bytes {
        report.skip(
            relative.to_owned(),
            SkipKind::Oversized,
            Some(format!("{bytes} bytes")),
        );
        return Ok(ChangedPath::MissingOrSkipped);
    }
    Ok(ChangedPath::Candidate(Box::new(ScannedFile {
        absolute: path,
        relative: relative.to_owned(),
        bytes,
        content_hash: None,
        content_fingerprint: None,
        version: from_metadata(&metadata),
        binary_checked: false,
    })))
}

fn ancestors_selected(
    root: &Path,
    relative: &str,
    options: &ScanOptions,
    matcher: &mut RepositoryMatcher,
    report: &mut ScanReport,
) -> Result<bool> {
    let relative_path = Path::new(relative);
    let Some(parent) = relative_path.parent() else {
        return Ok(true);
    };
    let mut directory = root.to_path_buf();
    for component in parent.components() {
        directory.push(component.as_os_str());
        let decision = matcher.matched(&directory, true)?;
        if skip_match(report, relative.to_owned(), decision) {
            return Ok(false);
        }
        if decision != RepositoryMatch::OverrideInclude
            && options.should_skip_directory(component.as_os_str())
        {
            report.skip(relative.to_owned(), SkipKind::StandardDirectory, None);
            return Ok(false);
        }
    }
    Ok(true)
}

fn local_error(
    path: &Path,
    relative: &str,
    operation: WalkOperation,
    source: io::Error,
    options: &ScanOptions,
    report: &mut ScanReport,
) -> Result<ChangedPath> {
    if options.walk.error_policy == crate::ErrorPolicy::Abort {
        return Err(Error::io(path, source));
    }
    let error = WalkError::new(
        path,
        Path::new(relative).components().count(),
        operation,
        source,
    );
    record_walk_error(&error, &report.root.clone(), report);
    Ok(ChangedPath::MissingOrSkipped)
}

fn is_safe_relative(relative: &str) -> bool {
    !relative.is_empty()
        && normalized_relative_path(Path::new(relative)) == relative
        && Path::new(relative).components().all(|component| {
            !matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
}
