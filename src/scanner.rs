use crate::config::{EvidenceMode, ScanOptions};
use crate::content::inspect_files;
use crate::error::{Error, Result};
use crate::ignore::{IgnoreRules, build_child_rules, skip_ignored};
use crate::path::normalized_relative_path;
use crate::report::{ScanReport, ScannedFile, SkipKind};
use crate::scan_finalize::finalize_report;
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
    let mut directory_rules = Vec::new();
    while let Some(item) = walker.next() {
        match item {
            Ok(entry) => process_entry(
                &entry,
                options,
                &mut report,
                &mut directory_rules,
                &mut walker,
            )?,
            Err(error) => record_walk_error(error, &canonical, options, &mut report)?,
        }
    }
    let inspected = inspect_files(std::mem::take(&mut report.files), options)?;
    report.files = inspected.files;
    report.skipped.extend(inspected.skipped);
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
    directory_rules: &mut Vec<(PathBuf, IgnoreRules)>,
    walker: &mut Walker,
) -> Result<()> {
    let relative_path = entry.relative_path();
    let relative = normalized_relative_path(relative_path);
    if entry.depth() == 0 {
        if let Some(reason) = entry.skip_reason() {
            report.skip(".".to_owned(), skip_kind(reason), None);
            return Ok(());
        }
        let rules = load_ignore_files(entry.path(), "", options, &IgnoreRules::default(), report)?;
        directory_rules.push((PathBuf::new(), rules));
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

    let parent = relative_path.parent().unwrap_or_else(|| Path::new(""));
    while directory_rules
        .last()
        .is_some_and(|(directory, _)| directory != parent)
    {
        directory_rules.pop();
    }
    if entry.is_dir() {
        if options.should_skip_directory(entry.file_name()) {
            walker.skip_current_dir();
            report.skip(relative, SkipKind::StandardDirectory, None);
            return Ok(());
        }
        let inherited_rules = directory_rules
            .last()
            .map(|(_, rules)| rules)
            .expect("root ignore rules are present");
        if skip_ignored(report, &relative, true, inherited_rules) {
            walker.skip_current_dir();
            return Ok(());
        }
        let rules = load_ignore_files(entry.path(), &relative, options, inherited_rules, report)?;
        directory_rules.push((relative_path.to_path_buf(), rules));
        return Ok(());
    }
    if !entry.is_file() {
        return Ok(());
    }
    let inherited_rules = directory_rules
        .last()
        .map(|(_, rules)| rules)
        .expect("root ignore rules are present");
    if skip_ignored(report, &relative, false, inherited_rules) {
        return Ok(());
    }
    process_file(entry, relative, options, report)
}

fn process_file(
    entry: &WalkEntry,
    relative: String,
    options: &ScanOptions,
    report: &mut ScanReport,
) -> Result<()> {
    let path = entry.path();
    if !options.accepts_extension(path) {
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

fn load_ignore_files(
    directory: &Path,
    relative: &str,
    options: &ScanOptions,
    inherited: &IgnoreRules,
    report: &mut ScanReport,
) -> Result<IgnoreRules> {
    let (rules, errors) = build_child_rules(
        directory,
        relative,
        &options.ignore_files,
        options.ignore_case_insensitive,
        inherited,
    );
    for source in errors {
        if options.walk.error_policy == ErrorPolicy::Abort {
            return Err(Error::io(
                directory,
                std::io::Error::new(source.kind(), source),
            ));
        }
        report.warn(
            Some(relative.to_owned()),
            format!("could not load ignore rules: {source}"),
        );
    }
    Ok(rules)
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
