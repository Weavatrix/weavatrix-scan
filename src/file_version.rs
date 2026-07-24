use crate::report::{FileIdentity, FileVersion};
use std::fs::{File, Metadata};
use std::time::UNIX_EPOCH;

pub(crate) fn from_metadata(metadata: &Metadata) -> FileVersion {
    FileVersion {
        modified_ns: metadata
            .modified()
            .ok()
            .and_then(|value| value.duration_since(UNIX_EPOCH).ok())
            .map(|value| value.as_nanos()),
        changed_ns: changed_ns(metadata),
        identity: metadata_identity(metadata),
    }
}

pub(crate) fn from_file(file: &File, metadata: &Metadata) -> std::io::Result<FileVersion> {
    let mut version = from_metadata(metadata);
    version.identity = file_identity(file, metadata)?;
    Ok(version)
}

pub(crate) fn reusable(previous: &FileVersion, current: &FileVersion) -> bool {
    previous.modified_ns.is_some()
        && previous.modified_ns == current.modified_ns
        && optional_equal(previous.changed_ns, current.changed_ns)
        && optional_equal(previous.identity, current.identity)
}

fn optional_equal<T: PartialEq>(previous: Option<T>, current: Option<T>) -> bool {
    match (previous, current) {
        (Some(previous), Some(current)) => previous == current,
        _ => true,
    }
}

#[cfg(unix)]
fn changed_ns(metadata: &Metadata) -> Option<u128> {
    use std::os::unix::fs::MetadataExt as _;
    let seconds = u128::try_from(metadata.ctime()).ok()?;
    let nanoseconds = u128::try_from(metadata.ctime_nsec()).ok()?;
    Some(seconds.saturating_mul(1_000_000_000) + nanoseconds)
}

#[cfg(not(unix))]
const fn changed_ns(_metadata: &Metadata) -> Option<u128> {
    None
}

#[cfg(unix)]
fn metadata_identity(metadata: &Metadata) -> Option<FileIdentity> {
    use std::os::unix::fs::MetadataExt as _;
    Some(FileIdentity {
        file_system: metadata.dev(),
        file: metadata.ino(),
    })
}

#[cfg(not(unix))]
const fn metadata_identity(_metadata: &Metadata) -> Option<FileIdentity> {
    None
}

#[cfg(windows)]
fn file_identity(file: &File, _metadata: &Metadata) -> std::io::Result<Option<FileIdentity>> {
    let information = winapi_util::file::information(file)?;
    Ok(Some(FileIdentity {
        file_system: information.volume_serial_number(),
        file: information.file_index(),
    }))
}

#[cfg(unix)]
fn file_identity(_file: &File, metadata: &Metadata) -> std::io::Result<Option<FileIdentity>> {
    Ok(metadata_identity(metadata))
}

#[cfg(not(any(unix, windows)))]
fn file_identity(_file: &File, _metadata: &Metadata) -> std::io::Result<Option<FileIdentity>> {
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::{from_file, from_metadata, reusable};

    #[test]
    fn stable_file_versions_are_reusable() {
        let path =
            std::env::temp_dir().join(format!("weavatrix-file-version-{}", std::process::id()));
        std::fs::write(&path, "stable").unwrap();
        let file = std::fs::File::open(&path).unwrap();
        let metadata = file.metadata().unwrap();
        let discovered = from_metadata(&metadata);
        let opened = from_file(&file, &metadata).unwrap();
        assert!(reusable(&discovered, &opened));
        let _ = std::fs::remove_file(path);
    }
}
