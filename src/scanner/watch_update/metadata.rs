use super::{Path, Result, ScanOptions, ScanReport, SkipKind, WalkOperation, fs, io, local_error};

pub(super) fn changed_metadata(
    path: &Path,
    root: &Path,
    relative: &str,
    options: &ScanOptions,
    report: &mut ScanReport,
) -> Result<Option<fs::Metadata>> {
    let link_metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(source) if source.kind() == io::ErrorKind::NotFound => {
            return Ok(None);
        }
        Err(source) => {
            return local_error(
                path,
                relative,
                WalkOperation::ReadMetadata,
                source,
                options,
                report,
            )
            .map(|_| None);
        }
    };
    let is_symlink = link_metadata.file_type().is_symlink();
    if is_symlink && !options.walk.follow_links {
        report.skip(relative.to_owned(), SkipKind::Symlink, None);
        return Ok(None);
    }
    let metadata = if is_symlink {
        let canonical = match path.canonicalize() {
            Ok(canonical) => canonical,
            Err(source) => {
                return local_error(
                    path,
                    relative,
                    WalkOperation::Canonicalize,
                    source,
                    options,
                    report,
                )
                .map(|_| None);
            }
        };
        if !canonical.starts_with(root) {
            report.skip(relative.to_owned(), SkipKind::PathEscape, None);
            return Ok(None);
        }
        match fs::metadata(path) {
            Ok(metadata) => metadata,
            Err(source) => {
                return local_error(
                    path,
                    relative,
                    WalkOperation::ReadMetadata,
                    source,
                    options,
                    report,
                )
                .map(|_| None);
            }
        }
    } else {
        link_metadata
    };
    Ok(Some(metadata))
}
