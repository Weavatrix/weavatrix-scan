use serde::Deserialize as _;
use serde::ser::SerializeMap as _;
use std::path::{Path, PathBuf};

pub(crate) fn serialize<S>(path: &Path, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    if let Some(text) = path.to_str() {
        return serializer.serialize_str(text);
    }

    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt as _;

        let mut map = serializer.serialize_map(Some(1))?;
        map.serialize_entry("unix_bytes", path.as_os_str().as_bytes())?;
        map.end()
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt as _;

        let units = path.as_os_str().encode_wide().collect::<Vec<_>>();
        let mut map = serializer.serialize_map(Some(1))?;
        map.serialize_entry("windows_wide", &units)?;
        map.end()
    }
    #[cfg(not(any(unix, windows)))]
    {
        Err(serde::ser::Error::custom(
            "non-Unicode paths are not supported on this platform",
        ))
    }
}

pub(crate) fn deserialize<'de, D>(deserializer: D) -> Result<PathBuf, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(serde::Deserialize)]
    #[serde(untagged)]
    enum EncodedPath {
        Text(String),
        #[cfg(unix)]
        Unix {
            unix_bytes: Vec<u8>,
        },
        #[cfg(windows)]
        Windows {
            windows_wide: Vec<u16>,
        },
    }

    match EncodedPath::deserialize(deserializer)? {
        EncodedPath::Text(text) => Ok(PathBuf::from(text)),
        #[cfg(unix)]
        EncodedPath::Unix { unix_bytes } => {
            use std::ffi::OsString;
            use std::os::unix::ffi::OsStringExt as _;

            Ok(PathBuf::from(OsString::from_vec(unix_bytes)))
        }
        #[cfg(windows)]
        EncodedPath::Windows { windows_wide } => {
            use std::ffi::OsString;
            use std::os::windows::ffi::OsStringExt as _;

            Ok(PathBuf::from(OsString::from_wide(&windows_wide)))
        }
    }
}
