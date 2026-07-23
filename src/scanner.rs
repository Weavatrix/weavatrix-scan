use crate::config::ScanOptions;
use crate::content::inspect_files;
use crate::error::{Error, Result};
use crate::ignore::{IgnoreRule, load_ignore_file, skip_ignored};
use crate::path::RevisionHasher;
use crate::report::{ScanReport, ScannedFile, SkipKind};
use std::ffi::OsStr;
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
    /// Returns an error when the root cannot be canonicalized/read or selected
    /// file metadata becomes unavailable during the scan.
    pub fn scan(self) -> Result<ScanReport> {
        scan_repository_with_options(&self.root, &self.options)
    }
}

/// Scans a repository with default options.
///
/// # Errors
///
/// Returns an error when the root cannot be canonicalized/read or selected file
/// metadata becomes unavailable during the scan.
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

    let mut report = ScanReport::new(canonical.clone());
    walk_directory(&canonical, &canonical, "", &[], options, &mut report)?;
    let inspected = inspect_files(std::mem::take(&mut report.files), options)?;
    report.files = inspected.files;
    report.skipped.extend(inspected.skipped);
    report
        .files
        .sort_by(|left, right| left.relative.cmp(&right.relative));
    report
        .skipped
        .sort_by(|left, right| left.relative.cmp(&right.relative));
    report.revision = revision_for(&report.files);
    Ok(report)
}

fn walk_directory(
    root: &Path,
    directory: &Path,
    relative_directory: &str,
    inherited_rules: &[IgnoreRule],
    options: &ScanOptions,
    report: &mut ScanReport,
) -> Result<()> {
    let mut entries = fs::read_dir(directory)
        .map_err(|source| Error::io(directory, source))?
        .collect::<std::io::Result<Vec<_>>>()
        .map_err(|source| Error::io(directory, source))?;
    entries.sort_by_key(std::fs::DirEntry::file_name);
    let mut rules = inherited_rules.to_vec();
    for ignore_name in &options.ignore_files {
        if let Some(entry) = entries
            .iter()
            .find(|entry| entry.file_name() == OsStr::new(ignore_name))
        {
            load_ignore_file(&entry.path(), relative_directory, &mut rules, report);
        }
    }

    for entry in entries {
        process_entry(&entry, root, relative_directory, &rules, options, report)?;
    }
    Ok(())
}

fn process_entry(
    entry: &fs::DirEntry,
    root: &Path,
    relative_directory: &str,
    rules: &[IgnoreRule],
    options: &ScanOptions,
    report: &mut ScanReport,
) -> Result<()> {
    let name = entry.file_name();
    let name_text = name.to_string_lossy();
    let relative = join_relative(relative_directory, &name_text);
    let file_type = entry
        .file_type()
        .map_err(|source| Error::io(entry.path(), source))?;
    if file_type.is_symlink() {
        report.skip(relative, SkipKind::Symlink, None);
        return Ok(());
    }
    if file_type.is_dir() {
        return process_directory(entry, root, &relative, rules, options, report);
    }
    if !file_type.is_file() || skip_ignored(report, &relative, false, rules) {
        return Ok(());
    }
    process_file(entry, root, relative, options, report)
}

fn process_directory(
    entry: &fs::DirEntry,
    root: &Path,
    relative: &str,
    rules: &[IgnoreRule],
    options: &ScanOptions,
    report: &mut ScanReport,
) -> Result<()> {
    if options.should_skip_directory(&entry.file_name().to_string_lossy()) {
        report.skip(relative.to_owned(), SkipKind::StandardDirectory, None);
        return Ok(());
    }
    if skip_ignored(report, relative, true, rules) {
        return Ok(());
    }
    let path = entry.path();
    if !path.starts_with(root) {
        report.skip(relative.to_owned(), SkipKind::PathEscape, None);
        return Ok(());
    }
    walk_directory(root, &path, relative, rules, options, report)
}

fn process_file(
    entry: &fs::DirEntry,
    root: &Path,
    relative: String,
    options: &ScanOptions,
    report: &mut ScanReport,
) -> Result<()> {
    let path = entry.path();
    if !options.accepts_extension(&path) {
        report.skip(relative, SkipKind::Extension, None);
        return Ok(());
    }
    if !path.starts_with(root) {
        report.skip(relative, SkipKind::PathEscape, None);
        return Ok(());
    }
    let metadata = entry
        .metadata()
        .map_err(|source| Error::io(&path, source))?;
    if metadata.len() > options.max_file_bytes {
        report.skip(
            relative,
            SkipKind::Oversized,
            Some(format!("{} bytes", metadata.len())),
        );
        return Ok(());
    }
    report.files.push(ScannedFile {
        absolute: path,
        relative,
        bytes: metadata.len(),
        content_hash: None,
    });
    Ok(())
}

fn join_relative(base: &str, name: &str) -> String {
    if base.is_empty() {
        name.to_owned()
    } else {
        format!("{base}/{name}")
    }
}

fn revision_for(files: &[ScannedFile]) -> String {
    let mut revision = RevisionHasher::new();
    for file in files {
        revision.write(file.relative.as_bytes());
        revision.write(&[0]);
        revision.write(file.content_hash.as_deref().unwrap_or("").as_bytes());
        revision.write(&[0xff]);
    }
    revision.finish()
}
