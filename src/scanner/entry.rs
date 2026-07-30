use crate::config::ScanOptions;
use crate::error::{Error, Result};
use crate::file_version::from_metadata;
use crate::ignore::{IgnoreRules, RepositoryMatch, RepositoryMatcher};
use crate::path::normalized_relative_path;
use crate::report::{FileVersion, ScanReport, ScannedFile, SkipKind};
use crate::scan_match::skip_match;
use crate::walk_types::{ErrorPolicy, WalkEntry, WalkError, WalkOperation, WalkSkipReason};
use std::fs;
use std::path::Path;

pub(super) fn prepare_batch_directory<'a>(
    matcher: &mut RepositoryMatcher,
    entries: &'a [WalkEntry],
) -> Result<Option<&'a Path>> {
    let Some(parent) = entries
        .first()
        .filter(|entry| entry.depth() > 0)
        .and_then(|entry| entry.path().parent())
    else {
        return Ok(None);
    };
    matcher.prepare_directory_from_entries(parent, entries)?;
    Ok(Some(parent))
}

pub(super) fn process_entry(
    entry: &WalkEntry,
    options: &ScanOptions,
    report: &mut ScanReport,
    matcher: &RepositoryMatcher,
    prepared_rules: Option<&IgnoreRules>,
) -> Result<bool> {
    let mut selected = None;
    let skip = process_entry_with(
        entry,
        options,
        report,
        matcher,
        prepared_rules,
        |path, relative, bytes, version| {
            selected = Some(ScannedFile {
                absolute: path.to_path_buf(),
                relative,
                bytes,
                content_hash: None,
                content_fingerprint: None,
                version,
                binary_checked: false,
            });
        },
    )?;
    if let Some(file) = selected {
        report.files.push(file);
    }
    Ok(skip)
}

pub(super) fn process_entry_with<F>(
    entry: &WalkEntry,
    options: &ScanOptions,
    report: &mut ScanReport,
    matcher: &RepositoryMatcher,
    prepared_rules: Option<&IgnoreRules>,
    mut selected: F,
) -> Result<bool>
where
    F: FnMut(&Path, String, u64, FileVersion),
{
    let relative_path = entry.relative_path();
    let relative = normalized_relative_path(relative_path);
    if entry.depth() == 0 {
        if let Some(reason) = entry.skip_reason() {
            report.skip(".".to_owned(), skip_kind(reason), None);
            return Ok(true);
        }
        return Ok(false);
    }
    if entry.depth() < options.effective_min_depth() && !entry.is_dir() {
        return Ok(false);
    }
    if entry.is_symlink() && !options.walk.follow_links {
        report.skip(relative, SkipKind::Symlink, None);
        return Ok(false);
    }
    if let Some(reason) = entry.skip_reason() {
        report.skip(relative, skip_kind(reason), None);
        return Ok(true);
    }

    if entry.is_dir() {
        let parent = entry.path().parent().unwrap_or(entry.path());
        let decision = prepared_rules.map_or_else(
            || {
                matcher.matched_prepared(
                    &relative,
                    parent,
                    entry.path(),
                    true,
                    entry.hidden(),
                    true,
                )
            },
            |rules| {
                matcher.matched_with_rules(
                    &relative,
                    entry.path(),
                    true,
                    entry.hidden(),
                    rules,
                    true,
                )
            },
        );
        if skip_match(report, &relative, decision) {
            return Ok(true);
        }
        if decision != RepositoryMatch::OverrideInclude
            && options.should_skip_directory(entry.file_name())
        {
            report.skip(relative, SkipKind::StandardDirectory, None);
            return Ok(true);
        }
        return Ok(false);
    }
    if !entry.is_file() {
        return Ok(false);
    }
    let parent = entry.path().parent().unwrap_or(entry.path());
    let decision = prepared_rules.map_or_else(
        || matcher.matched_prepared(&relative, parent, entry.path(), false, entry.hidden(), true),
        |rules| {
            matcher.matched_with_rules(&relative, entry.path(), false, entry.hidden(), rules, true)
        },
    );
    if skip_match(report, &relative, decision) {
        return Ok(false);
    }
    process_file(
        entry,
        relative,
        decision == RepositoryMatch::OverrideInclude,
        options,
        report,
        &mut selected,
    )?;
    Ok(false)
}

fn process_file<F>(
    entry: &WalkEntry,
    relative: String,
    override_include: bool,
    options: &ScanOptions,
    report: &mut ScanReport,
    selected: &mut F,
) -> Result<()>
where
    F: FnMut(&Path, String, u64, FileVersion),
{
    let path = entry.path();
    if !override_include && !options.accepts_extension(path, &relative) {
        report.skip(relative, SkipKind::Extension, None);
        return Ok(());
    }
    let (bytes, version) = match (entry.bytes(), entry.version()) {
        (Some(bytes), Some(version)) => (bytes, version),
        _ => match fs::metadata(path) {
            Ok(metadata) => (metadata.len(), from_metadata(&metadata)),
            Err(source) => {
                return record_local_io_error(
                    path,
                    relative,
                    WalkOperation::ReadMetadata,
                    source,
                    options,
                    report,
                );
            }
        },
    };
    if bytes > options.max_file_bytes {
        report.skip(
            relative,
            SkipKind::Oversized,
            Some(format!("{bytes} bytes")),
        );
        return Ok(());
    }
    selected(path, relative, bytes, version);
    Ok(())
}

pub(super) fn record_walk_error(error: &WalkError, root: &Path, report: &mut ScanReport) {
    let relative = error.path().strip_prefix(root).map_or_else(
        |_| normalized_relative_path(error.path()),
        normalized_relative_path,
    );
    let relative = if relative.is_empty() {
        ".".to_owned()
    } else {
        relative
    };
    let message = format!(
        "{}: {}",
        operation_label(error.operation()),
        error.io_error()
    );
    report.skip(relative.clone(), SkipKind::IoError, Some(message.clone()));
    report.warn(Some(relative), message);
}

fn record_local_io_error(
    path: &Path,
    relative: String,
    operation: WalkOperation,
    source: std::io::Error,
    options: &ScanOptions,
    report: &mut ScanReport,
) -> Result<()> {
    if options.walk.error_policy == ErrorPolicy::Abort {
        return Err(Error::io(path, source));
    }
    let message = format!("{}: {source}", operation_label(operation));
    report.skip(relative.clone(), SkipKind::IoError, Some(message.clone()));
    report.warn(Some(relative), message);
    Ok(())
}

const fn skip_kind(reason: WalkSkipReason) -> SkipKind {
    match reason {
        WalkSkipReason::MaxDepth => SkipKind::MaxDepth,
        WalkSkipReason::FileSystemBoundary => SkipKind::FileSystemBoundary,
        WalkSkipReason::PathEscape => SkipKind::PathEscape,
        WalkSkipReason::SymlinkLoop => SkipKind::SymlinkLoop,
    }
}

const fn operation_label(operation: WalkOperation) -> &'static str {
    match operation {
        WalkOperation::Canonicalize => "canonicalize",
        WalkOperation::ReadDirectory => "read directory",
        WalkOperation::ReadEntry => "read entry",
        WalkOperation::ReadMetadata => "read metadata",
        WalkOperation::ScheduleWorker => "schedule worker",
    }
}

pub(super) fn walker_error_into_scan_error(error: WalkError) -> Error {
    let (path, source) = error.into_parts();
    Error::io(path, source)
}
