mod decide;
mod walk;

use super::{Error, ParallelRuntime, Path, RepositoryMatcher, Result, ScanOptions, ScanRuntime};
use crate::report::ScanTermination;
use walk::{discover_parallel, discover_serial};

/// Collects sorted repository-relative paths without building a manifest.
///
/// Selection matches [`super::scan_repository`]: ignore rules, standard skips,
/// extensions, and hidden-file policy still apply. Paths use `/` on every
/// platform. File sizes, hashes, revision, descriptor, and skip evidence are
/// not collected, and `max_file_bytes` is not applied.
///
/// # Errors
///
/// Returns an error when the root cannot be resolved or a local I/O failure
/// occurs under `ErrorPolicy::Abort`. Cooperative cancel and timeout stop the
/// walk with an interrupted I/O error.
///
/// # Examples
///
/// ```
/// use weavatrix_scan::{ScanOptions, Scanner, scan_repository_paths};
///
/// let defaults = scan_repository_paths(".")?;
/// let rust_only = Scanner::new(".")
///     .options(ScanOptions::default().with_extensions(["rs"]))
///     .scan_paths()?;
/// assert!(defaults.iter().all(|path| !path.contains('\\')));
/// assert!(rust_only.iter().all(|path| path.ends_with(".rs")));
/// # Ok::<(), weavatrix_scan::Error>(())
/// ```
pub fn scan_repository_paths(root: impl AsRef<Path>) -> Result<Vec<String>> {
    scan_repository_paths_with_runtime(
        root.as_ref(),
        &ScanOptions::default(),
        &ParallelRuntime::global(),
    )
}

pub(crate) fn scan_repository_paths_with_runtime(
    root: &Path,
    options: &ScanOptions,
    parallel_runtime: &ParallelRuntime,
) -> Result<Vec<String>> {
    let mut paths = discover_paths(root, options, parallel_runtime)?;
    paths.sort_unstable();
    Ok(paths)
}

fn discover_paths(
    root: &Path,
    options: &ScanOptions,
    parallel_runtime: &ParallelRuntime,
) -> Result<Vec<String>> {
    let canonical = resolve_root(root, options)?;
    let matcher = RepositoryMatcher::with_options(&canonical, options)?;
    let runtime = ScanRuntime::new();
    let (paths, runtime) = if options.uses_parallel_traversal() {
        discover_parallel(&canonical, options, matcher, runtime, parallel_runtime)?
    } else {
        discover_serial(&canonical, options, matcher, runtime)?
    };
    if let Some(reason) = runtime.external_termination(options) {
        return Err(interrupted(&canonical, reason));
    }
    Ok(paths)
}

fn resolve_root(root: &Path, options: &ScanOptions) -> Result<std::path::PathBuf> {
    if options.walk.root_symlink_policy == crate::RootSymlinkPolicy::Reject {
        let metadata = std::fs::symlink_metadata(root).map_err(|source| Error::io(root, source))?;
        if metadata.file_type().is_symlink() {
            return Err(Error::io(
                root,
                std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "root symlink rejected by policy",
                ),
            ));
        }
    }
    let canonical = root
        .canonicalize()
        .map_err(|source| Error::io(root, source))?;
    if canonical.is_dir() {
        Ok(canonical)
    } else {
        Err(Error::InvalidRoot(canonical))
    }
}

pub(super) fn finish(
    root: &Path,
    paths: Vec<String>,
    runtime: ScanRuntime,
    reason: ScanTermination,
) -> Result<(Vec<String>, ScanRuntime)> {
    if let Some(error) = stop_error(root, reason) {
        Err(error)
    } else {
        Ok((paths, runtime))
    }
}

pub(super) fn stop_error(root: &Path, reason: ScanTermination) -> Option<Error> {
    match reason {
        ScanTermination::Cancelled | ScanTermination::Timeout => Some(interrupted(root, reason)),
        ScanTermination::MaxEntries | ScanTermination::MaxTotalBytes => None,
    }
}

pub(super) fn interrupted(root: &Path, reason: ScanTermination) -> Error {
    let message = match reason {
        ScanTermination::Cancelled => "scan cancelled",
        ScanTermination::Timeout => "scan timed out",
        ScanTermination::MaxEntries => "entry limit reached",
        ScanTermination::MaxTotalBytes => "byte limit reached",
    };
    Error::io(
        root,
        std::io::Error::new(std::io::ErrorKind::Interrupted, message),
    )
}
