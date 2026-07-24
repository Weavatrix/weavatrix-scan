use crate::report::FileIdentity;
use std::io;
use std::path::Path;

pub(crate) fn identity() -> Option<FileIdentity> {
    platform_stdout_identity()
}

pub(crate) fn path_matches(path: &Path, expected: FileIdentity) -> io::Result<bool> {
    platform_path_identity(path).map(|actual| actual == Some(expected))
}

#[cfg(unix)]
fn platform_stdout_identity() -> Option<FileIdentity> {
    std::fs::metadata("/dev/stdout")
        .or_else(|_| std::fs::metadata("/proc/self/fd/1"))
        .ok()
        .filter(std::fs::Metadata::is_file)
        .and_then(|metadata| metadata_identity(&metadata))
}

#[cfg(unix)]
fn platform_path_identity(path: &Path) -> io::Result<Option<FileIdentity>> {
    std::fs::metadata(path).map(|metadata| metadata_identity(&metadata))
}

#[cfg(unix)]
#[expect(
    clippy::unnecessary_wraps,
    reason = "the target-specific helpers intentionally share one portable return shape"
)]
fn metadata_identity(metadata: &std::fs::Metadata) -> Option<FileIdentity> {
    use std::os::unix::fs::MetadataExt as _;
    Some(FileIdentity {
        file_system: metadata.dev(),
        file: metadata.ino(),
    })
}

#[cfg(windows)]
fn platform_stdout_identity() -> Option<FileIdentity> {
    let handle = winapi_util::HandleRef::stdout();
    if !winapi_util::file::typ(&handle).ok()?.is_disk() {
        return None;
    }
    let information = winapi_util::file::information(&handle).ok()?;
    Some(FileIdentity {
        file_system: information.volume_serial_number(),
        file: information.file_index(),
    })
}

#[cfg(windows)]
fn platform_path_identity(path: &Path) -> io::Result<Option<FileIdentity>> {
    let handle = winapi_util::Handle::from_path(path)?;
    let information = winapi_util::file::information(&handle)?;
    Ok(Some(FileIdentity {
        file_system: information.volume_serial_number(),
        file: information.file_index(),
    }))
}

#[cfg(not(any(unix, windows)))]
const fn platform_stdout_identity() -> Option<FileIdentity> {
    None
}

#[cfg(not(any(unix, windows)))]
fn platform_path_identity(_path: &Path) -> io::Result<Option<FileIdentity>> {
    Ok(None)
}
