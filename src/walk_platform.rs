use std::fs::Metadata;
use std::io;
use std::path::Path;

use crate::walk_types::{DirectoryIdentity, FileSystemId};

#[derive(Debug, Clone, Copy)]
pub(crate) struct PlatformDirectoryInfo {
    pub(crate) file_system: FileSystemId,
    pub(crate) identity: DirectoryIdentity,
}

#[cfg(unix)]
#[allow(clippy::unnecessary_wraps)] // Windows identity lookup is genuinely fallible.
pub(crate) fn directory_info(
    _canonical: &Path,
    metadata: &Metadata,
) -> io::Result<PlatformDirectoryInfo> {
    use std::os::unix::fs::MetadataExt as _;
    let file_system = FileSystemId(metadata.dev());
    Ok(PlatformDirectoryInfo {
        file_system,
        identity: DirectoryIdentity {
            file_system,
            file: metadata.ino(),
        },
    })
}

#[cfg(windows)]
pub(crate) fn directory_info(
    canonical: &Path,
    _metadata: &Metadata,
) -> io::Result<PlatformDirectoryInfo> {
    let handle = winapi_util::Handle::from_path_any(canonical)?;
    let information = winapi_util::file::information(&handle)?;
    let file_system = FileSystemId(information.volume_serial_number());
    Ok(PlatformDirectoryInfo {
        file_system,
        identity: DirectoryIdentity {
            file_system,
            file: information.file_index(),
        },
    })
}

#[cfg(not(any(unix, windows)))]
pub(crate) fn directory_info(
    _canonical: &Path,
    _metadata: &Metadata,
) -> io::Result<PlatformDirectoryInfo> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "filesystem identity is unsupported on this platform",
    ))
}
