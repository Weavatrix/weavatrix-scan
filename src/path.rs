use std::ffi::OsStr;
#[cfg(any(unix, windows))]
use std::fmt::Write as _;
use std::path::{Component, Path};

pub(crate) fn looks_binary(bytes: &[u8]) -> bool {
    bytes.contains(&0)
}

pub(crate) fn normalized_relative_path(path: &Path) -> String {
    let mut normalized = String::new();
    for component in path.components() {
        let value = match component {
            Component::Normal(value) => encode_component(value),
            Component::ParentDir => "..".to_owned(),
            Component::CurDir | Component::RootDir | Component::Prefix(_) => continue,
        };
        if !normalized.is_empty() {
            normalized.push('/');
        }
        normalized.push_str(&value);
    }
    normalized
}

#[cfg(unix)]
fn encode_component(value: &OsStr) -> String {
    use std::os::unix::ffi::OsStrExt as _;

    let bytes = value.as_bytes();
    let mut encoded = String::new();
    let mut offset = 0;
    while offset < bytes.len() {
        match std::str::from_utf8(&bytes[offset..]) {
            Ok(valid) => {
                push_valid_text(&mut encoded, valid);
                break;
            }
            Err(error) => {
                let valid_end = offset + error.valid_up_to();
                if valid_end > offset {
                    let valid = std::str::from_utf8(&bytes[offset..valid_end])
                        .expect("UTF-8 validator supplied a valid prefix");
                    push_valid_text(&mut encoded, valid);
                }
                let invalid_len = error.error_len().unwrap_or(bytes.len() - valid_end);
                for byte in &bytes[valid_end..valid_end + invalid_len] {
                    let _ = write!(encoded, "%{byte:02X}");
                }
                offset = valid_end + invalid_len;
            }
        }
    }
    encoded
}

#[cfg(windows)]
fn encode_component(value: &OsStr) -> String {
    use std::os::windows::ffi::OsStrExt as _;

    let units = value.encode_wide().collect::<Vec<_>>();
    let mut encoded = String::new();
    let mut index = 0;
    while index < units.len() {
        let unit = units[index];
        if (0xd800..=0xdbff).contains(&unit)
            && units
                .get(index + 1)
                .is_some_and(|next| (0xdc00..=0xdfff).contains(next))
        {
            let next = units[index + 1];
            let scalar = 0x1_0000 + ((u32::from(unit) - 0xd800) << 10) + (u32::from(next) - 0xdc00);
            push_char(
                &mut encoded,
                char::from_u32(scalar).expect("valid surrogate pair"),
            );
            index += 2;
        } else if (0xd800..=0xdfff).contains(&unit) {
            let _ = write!(encoded, "%u{unit:04X}");
            index += 1;
        } else {
            push_char(
                &mut encoded,
                char::from_u32(u32::from(unit)).expect("non-surrogate UTF-16 unit"),
            );
            index += 1;
        }
    }
    encoded
}

#[cfg(not(any(unix, windows)))]
fn encode_component(value: &OsStr) -> String {
    value.to_string_lossy().replace('%', "%25")
}

#[cfg(unix)]
fn push_valid_text(output: &mut String, text: &str) {
    for character in text.chars() {
        push_char(output, character);
    }
}

fn push_char(output: &mut String, character: char) {
    if character == '%' {
        output.push_str("%25");
    } else {
        output.push(character);
    }
}

pub(crate) struct RevisionHasher(u64);

impl RevisionHasher {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;

    pub(crate) const fn new() -> Self {
        Self(Self::OFFSET)
    }

    pub(crate) fn write(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.0 ^= u64::from(*byte);
            self.0 = self.0.wrapping_mul(Self::PRIME);
        }
    }

    pub(crate) fn finish(self) -> String {
        format!("fnv1a64:{:016x}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::normalized_relative_path;
    use std::path::Path;

    #[test]
    fn normalizes_separators_and_escapes_percent_without_loss() {
        assert_eq!(
            normalized_relative_path(Path::new("src").join("100%").join("lib.rs").as_path()),
            "src/100%25/lib.rs"
        );
    }
}
