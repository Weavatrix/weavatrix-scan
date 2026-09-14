use std::collections::BTreeSet;
use std::ffi::OsStr;
#[cfg(any(unix, windows))]
use std::fmt::Write as _;
use std::path::{Component, Path};

/// Returns whether `path` is `prefix` or a descendant, using `/` component
/// boundaries. `src` matches `src/a.rs` and does not match `src2`.
#[must_use]
pub fn is_same_or_descendant(path: &str, prefix: &str) -> bool {
    path == prefix
        || (!prefix.is_empty()
            && path.len() > prefix.len()
            && path.as_bytes().get(prefix.len()) == Some(&b'/')
            && path.starts_with(prefix))
}

/// Returns whether `path` is covered by any collapsed prefix in `prefixes`.
///
/// Ancestor lookup is `O(depth · log k)` after prefixes have been collapsed.
#[must_use]
pub fn path_covered_by_prefixes(path: &str, prefixes: &BTreeSet<&str>) -> bool {
    if prefixes.contains(path) {
        return true;
    }
    let mut rest = path;
    while let Some((parent, _)) = rest.rsplit_once('/') {
        if prefixes.contains(parent) {
            return true;
        }
        rest = parent;
    }
    false
}

/// Deduplicates and drops prefixes already covered by a shorter ancestor.
///
/// `src` and `src/nested` collapse to `src`. `src` and `src2` stay distinct.
#[must_use]
pub fn collapse_path_prefixes<'a, I>(prefixes: I) -> BTreeSet<&'a str>
where
    I: IntoIterator<Item = &'a str>,
{
    let mut sorted = prefixes.into_iter().collect::<Vec<_>>();
    sorted.sort_unstable();
    sorted.dedup();
    let mut collapsed = BTreeSet::new();
    for prefix in sorted {
        if !path_covered_by_prefixes(prefix, &collapsed) {
            collapsed.insert(prefix);
        }
    }
    collapsed
}

pub(crate) fn normalized_relative_path(path: &Path) -> String {
    if path.is_relative()
        && path
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
        && let Some(text) = path.to_str()
    {
        return normalize_valid_text(text);
    }
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

fn normalize_valid_text(text: &str) -> String {
    if !text.contains('%') && !text.contains(std::path::MAIN_SEPARATOR) {
        return text.to_owned();
    }
    let mut normalized = String::with_capacity(text.len());
    for character in text.chars() {
        if character == '%' {
            normalized.push_str("%25");
        } else if character == std::path::MAIN_SEPARATOR {
            normalized.push('/');
        } else {
            normalized.push(character);
        }
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

#[cfg(test)]
mod tests {
    use std::path::Path;

    #[test]
    fn normalizes_separators_and_escapes_percent_without_loss() {
        assert_eq!(
            super::normalized_relative_path(Path::new("src").join("100%").join("lib.rs").as_path(),),
            "src/100%25/lib.rs"
        );
    }

    #[test]
    fn descendant_check_uses_component_boundaries() {
        assert!(super::is_same_or_descendant("src", "src"));
        assert!(super::is_same_or_descendant("src/nested/a.rs", "src"));
        assert!(!super::is_same_or_descendant("src2", "src"));
        assert!(!super::is_same_or_descendant("src", "src/nested"));
    }

    #[test]
    fn collapsed_prefixes_keep_component_boundaries() {
        let prefixes = super::collapse_path_prefixes(["src/nested", "src", "src", "src2"]);
        assert_eq!(
            prefixes.iter().copied().collect::<Vec<_>>(),
            ["src", "src2"]
        );
        assert!(super::path_covered_by_prefixes("src/a.rs", &prefixes));
        assert!(super::path_covered_by_prefixes("src2/b.rs", &prefixes));
        assert!(!super::path_covered_by_prefixes("src3/c.rs", &prefixes));
    }
}
