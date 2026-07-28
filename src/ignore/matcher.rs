use crate::glob;

#[derive(Debug, Clone)]
pub(super) enum RuleMatcher {
    Literal(bool),
    Prefix(String, bool),
    Suffix(String, bool),
    Glob {
        needle: Option<String>,
        prefix: Option<String>,
        suffix: Option<String>,
        case_insensitive: bool,
    },
}

impl RuleMatcher {
    pub(super) fn new(pattern: &str, case_insensitive: bool) -> Self {
        if pattern.contains('{') || pattern.contains("**") {
            Self::glob(pattern, case_insensitive)
        } else if !has_meta(pattern) {
            Self::Literal(case_insensitive)
        } else if let Some(suffix) = pattern.strip_prefix('*').filter(|value| !has_meta(value)) {
            Self::Suffix(suffix.to_owned(), case_insensitive)
        } else if let Some(prefix) = pattern.strip_suffix('*').filter(|value| !has_meta(value)) {
            Self::Prefix(prefix.to_owned(), case_insensitive)
        } else {
            Self::glob(pattern, case_insensitive)
        }
    }

    fn glob(pattern: &str, case_insensitive: bool) -> Self {
        Self::Glob {
            needle: (!pattern.contains('{') && !pattern.contains("**"))
                .then(|| longest_literal(pattern))
                .flatten(),
            prefix: edge_literal(pattern, true),
            suffix: edge_literal(pattern, false),
            case_insensitive,
        }
    }

    pub(super) const fn is_literal(&self) -> bool {
        matches!(self, Self::Literal(false))
    }

    pub(super) fn prefix_key(&self, pattern: &str) -> Option<u8> {
        match self {
            Self::Prefix(prefix, false) => prefix.bytes().next(),
            Self::Glob {
                prefix,
                case_insensitive: false,
                ..
            } => prefix.as_ref().and_then(|prefix| prefix.bytes().next()),
            Self::Literal(false) => pattern.bytes().next(),
            _ => None,
        }
    }

    pub(super) fn suffix_key(&self, pattern: &str) -> Option<u8> {
        match self {
            Self::Suffix(suffix, false) => suffix.bytes().next_back(),
            Self::Literal(false) => pattern.bytes().next_back(),
            _ => None,
        }
    }

    pub(super) fn matches(&self, pattern: &str, value: &str) -> bool {
        match self {
            Self::Literal(false) => pattern == value,
            Self::Literal(true) => pattern.eq_ignore_ascii_case(value),
            Self::Prefix(prefix, false) => value.starts_with(prefix),
            Self::Suffix(suffix, false) => value.ends_with(suffix),
            Self::Glob {
                needle,
                prefix,
                suffix,
                case_insensitive: false,
            } => {
                prefix
                    .as_ref()
                    .is_none_or(|prefix| value.starts_with(prefix))
                    && suffix.as_ref().is_none_or(|suffix| value.ends_with(suffix))
                    && needle.as_ref().is_none_or(|needle| value.contains(needle))
                    && glob::matches(pattern, value)
            }
            Self::Prefix(prefix, true) => value
                .get(..prefix.len())
                .is_some_and(|value| value.eq_ignore_ascii_case(prefix)),
            Self::Suffix(suffix, true) => value
                .get(value.len().saturating_sub(suffix.len())..)
                .is_some_and(|value| value.eq_ignore_ascii_case(suffix)),
            Self::Glob {
                needle,
                prefix,
                suffix,
                case_insensitive: true,
            } => {
                let value = value.to_ascii_lowercase();
                prefix
                    .as_ref()
                    .is_none_or(|prefix| value.starts_with(prefix))
                    && suffix.as_ref().is_none_or(|suffix| value.ends_with(suffix))
                    && needle.as_ref().is_none_or(|needle| value.contains(needle))
                    && glob::matches(pattern, &value)
            }
        }
    }
}

fn has_meta(value: &str) -> bool {
    value
        .bytes()
        .any(|byte| matches!(byte, b'*' | b'?' | b'[' | b'{' | b'\\'))
}

fn longest_literal(pattern: &str) -> Option<String> {
    let mut longest = Vec::new();
    let mut current = Vec::new();
    let mut bytes = pattern.bytes();
    while let Some(byte) = bytes.next() {
        match byte {
            b'\\' => {
                if let Some(escaped) = bytes.next() {
                    current.push(escaped);
                }
            }
            b'[' => {
                retain_longest(&mut current, &mut longest);
                for member in bytes.by_ref() {
                    if member == b']' {
                        break;
                    }
                }
            }
            b'*' | b'?' | b'{' | b'}' | b',' => {
                retain_longest(&mut current, &mut longest);
            }
            literal => current.push(literal),
        }
    }
    retain_longest(&mut current, &mut longest);
    (longest.len() >= 2)
        .then(|| String::from_utf8(longest).ok())
        .flatten()
}

fn edge_literal(pattern: &str, prefix: bool) -> Option<String> {
    let mut edge = Vec::new();
    let mut current = Vec::new();
    let mut bytes = pattern.bytes();
    while let Some(byte) = bytes.next() {
        match byte {
            b'\\' => {
                if let Some(escaped) = bytes.next() {
                    current.push(escaped);
                }
            }
            b'[' => {
                if prefix {
                    break;
                }
                current.clear();
                for member in bytes.by_ref() {
                    if member == b']' {
                        break;
                    }
                }
            }
            b'*' | b'?' | b'{' | b'}' | b',' => {
                if prefix {
                    break;
                }
                current.clear();
            }
            literal => current.push(literal),
        }
    }
    if prefix {
        edge = current;
    } else {
        std::mem::swap(&mut edge, &mut current);
        if pattern.contains("**/") && edge.first() == Some(&b'/') {
            edge.remove(0);
        }
    }
    (!edge.is_empty())
        .then(|| String::from_utf8(edge).ok())
        .flatten()
}

fn retain_longest(current: &mut Vec<u8>, longest: &mut Vec<u8>) {
    if current.len() > longest.len() {
        std::mem::swap(current, longest);
    }
    current.clear();
}

#[cfg(test)]
mod tests {
    use super::RuleMatcher;

    #[test]
    fn specializes_literal_prefix_suffix_and_complex_patterns() {
        assert!(RuleMatcher::new("Cargo.toml", false).is_literal());
        assert!(RuleMatcher::new("target*", false).matches("target*", "target-debug"));
        assert!(RuleMatcher::new("*.rs", false).matches("*.rs", "lib.rs"));
        let complex = RuleMatcher::new("report.[0-9]*.json", false);
        assert!(complex.matches("report.[0-9]*.json", "report.1.json"));
        assert!(!complex.matches("report.[0-9]*.json", "notes.json"));
    }
}
