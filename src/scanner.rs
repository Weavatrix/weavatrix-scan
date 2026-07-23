use crate::config::{EvidenceMode, ScanOptions};
use crate::content::inspect_files;
use crate::error::{Error, Result};
use crate::ignore::{RepositoryMatch, RepositoryMatcher};
use crate::path::normalized_relative_path;
use crate::report::{ScanReport, ScannedFile, SkipKind};
use crate::scan_finalize::finalize_report;
use crate::scan_limits::{ScanRuntime, apply_total_bytes_limit};
use crate::scan_match::skip_match;
use crate::walker::{ErrorPolicy, WalkEntry, WalkError, WalkOperation, WalkSkipReason, Walker};
use std::fs;
use std::path::{Path, PathBuf};

pub struct Scanner {
    root: PathBuf,
    options: ScanOptions,
}

impl Scanner {
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            options: ScanOptions::default(),
        }
    }

    #[must_use]
    pub fn options(mut self, options: ScanOptions) -> Self {
        self.options = options;
        self
    }

    /// Scans the configured repository root.
    ///
    /// # Errors
    ///
    /// Returns an error when the root cannot be canonicalized/read, or when a
    /// local error occurs under `ErrorPolicy::Abort`.
    pub fn scan(self) -> Result<ScanReport> {
        scan_repository_with_options(&self.root, &self.options)
    }
}

/// Scans a repository with default options.
///
/// # Errors
///
/// Returns an error when the root cannot be canonicalized/read, or when a local
/// error occurs under `ErrorPolicy::Abort`.
pub fn scan_repository(root: impl AsRef<Path>) -> Result<ScanReport> {
    scan_repository_with_options(root.as_ref(), &ScanOptions::default())
}

fn scan_repository_with_options(root: &Path, options: &ScanOptions) -> Result<ScanReport> {
    let canonical = root
        .canonicalize()
        .map_err(|source| Error::io(root, source))?;
    if !canonical.is_dir() {
        return Err(Error::InvalidRoot(canonical));
    }

    let mut report = ScanReport::new(
        canonical.clone(),
        options.evidence == EvidenceMode::Complete,
    );
    let mut walker = Walker::with_options(&canonical, options.walk_options())
        .map_err(walker_error_into_scan_error)?;
    let mut matcher = RepositoryMatcher::with_options(&canonical, options)?;
    let mut runtime = ScanRuntime::new();
    loop {
        if let Some(reason) = runtime.before_next(options) {
            report.terminate(reason);
            break;
        }
        let Some(item) = walker.next() else {
            break;
        };
        runtime.record_entry();
        match item {
            Ok(entry) => {
                process_entry(&entry, options, &mut report, &mut matcher, &mut walker)?;
            }
            Err(error) => record_walk_error(error, &canonical, options, &mut report)?,
        }
    }
    report.ignore_sources = matcher.sources().to_vec();
    report.portable = matcher.portable();
    if !matcher.warnings().is_empty() {
        report.complete = false;
        report.warnings.extend_from_slice(matcher.warnings());
    }
    if report.termination.is_none()
        && let Some(reason) = runtime.external_termination(options)
    {
        report.terminate(reason);
    }
    apply_total_bytes_limit(&mut report, options);
    let inspected = inspect_files(std::mem::take(&mut report.files), options, runtime.started)?;
    report.files = inspected.files;
    report.skipped.extend(inspected.skipped);
    if let Some(reason) = inspected.termination {
        report.terminate(reason);
    }
    if !inspected.warnings.is_empty() {
        report.complete = false;
        report.warnings.extend(inspected.warnings);
    }
    finalize_report(&mut report);
    Ok(report)
}

fn process_entry(
    entry: &WalkEntry,
    options: &ScanOptions,
    report: &mut ScanReport,
    matcher: &mut RepositoryMatcher,
    walker: &mut Walker,
) -> Result<()> {
    let relative_path = entry.relative_path();
    let relative = normalized_relative_path(relative_path);
    if entry.depth() == 0 {
        if let Some(reason) = entry.skip_reason() {
            report.skip(".".to_owned(), skip_kind(reason), None);
            return Ok(());
        }
        matcher.prepare_directory(entry.path())?;
        return Ok(());
    }
    if entry.depth() < options.effective_min_depth() && !entry.is_dir() {
        return Ok(());
    }
    if entry.is_symlink() && !options.walk.follow_links {
        report.skip(relative, SkipKind::Symlink, None);
        return Ok(());
    }
    if let Some(reason) = entry.skip_reason() {
        report.skip(relative, skip_kind(reason), None);
        return Ok(());
    }

    if entry.is_dir() {
        let parent = entry.path().parent().unwrap_or(entry.path());
        let decision = matcher.matched_prepared(&relative, parent, entry.path(), true);
        if skip_match(report, relative.clone(), decision) {
            walker.skip_current_dir();
            return Ok(());
        }
        if decision != RepositoryMatch::OverrideInclude
            && options.should_skip_directory(entry.file_name())
        {
            walker.skip_current_dir();
            report.skip(relative, SkipKind::StandardDirectory, None);
            return Ok(());
        }
        matcher.prepare_directory(entry.path())?;
        return Ok(());
    }
    if !entry.is_file() {
        return Ok(());
    }
    let parent = entry.path().parent().unwrap_or(entry.path());
    let decision = matcher.matched_prepared(&relative, parent, entry.path(), false);
    if skip_match(report, relative.clone(), decision) {
        return Ok(());
    }
    process_file(
        entry,
        relative,
        decision == RepositoryMatch::OverrideInclude,
        options,
        report,
    )
}

fn process_file(
    entry: &WalkEntry,
    relative: String,
    override_include: bool,
    options: &ScanOptions,
    report: &mut ScanReport,
) -> Result<()> {
    let path = entry.path();
    if !override_include && !options.accepts_extension(path) {
        report.skip(relative, SkipKind::Extension, None);
        return Ok(());
    }
    let bytes = match entry.bytes() {
        Some(bytes) => bytes,
        None => match fs::metadata(path) {
            Ok(metadata) => metadata.len(),
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
    report.files.push(ScannedFile {
        absolute: path.to_path_buf(),
        relative,
        bytes,
        content_hash: None,
    });
    Ok(())
}

fn record_walk_error(
    error: WalkError,
    root: &Path,
    options: &ScanOptions,
    report: &mut ScanReport,
) -> Result<()> {
    if options.walk.error_policy == ErrorPolicy::Abort {
        return Err(walker_error_into_scan_error(error));
    }
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
    Ok(())
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
    }
}

fn walker_error_into_scan_error(error: WalkError) -> Error {
    let (path, source) = error.into_parts();
    Error::io(path, source)
}
