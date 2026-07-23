use crate::glob;

#[derive(Debug, Clone)]
pub(super) enum RuleMatcher {
    Literal,
    Prefix(String),
    Suffix(String),
    Glob(Option<String>),
}

impl RuleMatcher {
    pub(super) fn new(pattern: &str) -> Self {
        if !has_meta(pattern) {
            Self::Literal
        } else if let Some(suffix) = pattern.strip_prefix('*').filter(|value| !has_meta(value)) {
            Self::Suffix(suffix.to_owned())
        } else if let Some(prefix) = pattern.strip_suffix('*').filter(|value| !has_meta(value)) {
            Self::Prefix(prefix.to_owned())
        } else {
            Self::Glob(longest_literal(pattern))
        }
    }

    pub(super) const fn is_literal(&self) -> bool {
        matches!(self, Self::Literal)
    }

    pub(super) fn matches(&self, pattern: &str, value: &str) -> bool {
        match self {
            Self::Literal => pattern == value,
            Self::Prefix(prefix) => value.starts_with(prefix),
            Self::Suffix(suffix) => value.ends_with(suffix),
            Self::Glob(needle) => {
                needle.as_ref().is_none_or(|needle| value.contains(needle))
                    && glob::matches(pattern, value)
            }
        }
    }
}

fn has_meta(value: &str) -> bool {
    value
        .bytes()
        .any(|byte| matches!(byte, b'*' | b'?' | b'[' | b'\\'))
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
            b'*' | b'?' => retain_longest(&mut current, &mut longest),
            literal => current.push(literal),
        }
    }
    retain_longest(&mut current, &mut longest);
    (longest.len() >= 2)
        .then(|| String::from_utf8(longest).ok())
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
        assert!(RuleMatcher::new("Cargo.toml").is_literal());
        assert!(RuleMatcher::new("target*").matches("target*", "target-debug"));
        assert!(RuleMatcher::new("*.rs").matches("*.rs", "lib.rs"));
        let complex = RuleMatcher::new("report.[0-9]*.json");
        assert!(complex.matches("report.[0-9]*.json", "report.1.json"));
        assert!(!complex.matches("report.[0-9]*.json", "notes.json"));
    }
}
