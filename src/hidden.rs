use std::path::Path;

pub(crate) fn is_hidden(path: &Path) -> bool {
    if path.file_name().is_some_and(name_starts_with_dot) {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt as _;

        std::fs::metadata(path).is_ok_and(|metadata| {
            winapi_util::file::is_hidden(u64::from(metadata.file_attributes()))
        })
    }
    #[cfg(not(windows))]
    false
}

#[cfg(unix)]
fn name_starts_with_dot(name: &std::ffi::OsStr) -> bool {
    use std::os::unix::ffi::OsStrExt as _;

    name.as_bytes().first() == Some(&b'.')
}

#[cfg(windows)]
fn name_starts_with_dot(name: &std::ffi::OsStr) -> bool {
    use std::os::windows::ffi::OsStrExt as _;

    name.encode_wide().next() == Some(u16::from(b'.'))
}

#[cfg(not(any(unix, windows)))]
fn name_starts_with_dot(name: &std::ffi::OsStr) -> bool {
    name.to_str().is_some_and(|name| name.starts_with('.'))
}
