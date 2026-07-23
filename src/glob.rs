pub(crate) fn matches(pattern: &str, value: &str) -> bool {
    let pattern = pattern.as_bytes();
    let value = value.as_bytes();
    let width = value.len() + 1;
    let mut memo = vec![0; (pattern.len() + 1) * width];
    matches_at(pattern, value, 0, 0, width, &mut memo)
}

fn matches_at(
    pattern: &[u8],
    value: &[u8],
    pattern_index: usize,
    value_index: usize,
    width: usize,
    memo: &mut [u8],
) -> bool {
    let slot = pattern_index * width + value_index;
    match memo[slot] {
        1 => return false,
        2 => return true,
        _ => {}
    }
    let result = match pattern.get(pattern_index) {
        None => value_index == value.len(),
        Some(b'*') if pattern.get(pattern_index + 1) == Some(&b'*') => {
            double_star(pattern, value, pattern_index, value_index, width, memo)
        }
        Some(b'*') => {
            matches_at(pattern, value, pattern_index + 1, value_index, width, memo)
                || (value.get(value_index).is_some_and(|byte| *byte != b'/')
                    && matches_at(pattern, value, pattern_index, value_index + 1, width, memo))
        }
        Some(b'?') => {
            value.get(value_index).is_some_and(|byte| *byte != b'/')
                && matches_at(
                    pattern,
                    value,
                    pattern_index + 1,
                    value_index + 1,
                    width,
                    memo,
                )
        }
        Some(b'[') => character_class(pattern, value, pattern_index, value_index).is_some_and(
            |(class_matches, next_pattern)| {
                class_matches
                    && matches_at(pattern, value, next_pattern, value_index + 1, width, memo)
            },
        ),
        Some(b'\\') if pattern_index + 1 < pattern.len() => {
            value.get(value_index) == pattern.get(pattern_index + 1)
                && matches_at(
                    pattern,
                    value,
                    pattern_index + 2,
                    value_index + 1,
                    width,
                    memo,
                )
        }
        Some(literal) => {
            value.get(value_index) == Some(literal)
                && matches_at(
                    pattern,
                    value,
                    pattern_index + 1,
                    value_index + 1,
                    width,
                    memo,
                )
        }
    };
    memo[slot] = if result { 2 } else { 1 };
    result
}

fn double_star(
    pattern: &[u8],
    value: &[u8],
    pattern_index: usize,
    value_index: usize,
    width: usize,
    memo: &mut [u8],
) -> bool {
    let mut next = pattern_index + 2;
    while pattern.get(next) == Some(&b'*') {
        next += 1;
    }
    let skip = if pattern.get(next) == Some(&b'/') {
        matches_at(pattern, value, next + 1, value_index, width, memo)
    } else {
        matches_at(pattern, value, next, value_index, width, memo)
    };
    skip || (value_index < value.len()
        && matches_at(pattern, value, pattern_index, value_index + 1, width, memo))
}

fn character_class(
    pattern: &[u8],
    value: &[u8],
    pattern_index: usize,
    value_index: usize,
) -> Option<(bool, usize)> {
    let candidate = *value.get(value_index)?;
    if candidate == b'/' {
        return None;
    }
    let mut index = pattern_index + 1;
    let negated = matches!(pattern.get(index), Some(b'!' | b'^'));
    index += usize::from(negated);
    let mut matched = false;
    let mut has_member = false;
    while let Some(member) = pattern.get(index).copied() {
        if member == b']' && has_member {
            return Some((matched != negated, index + 1));
        }
        has_member = true;
        let (start, consumed) = escaped_member(pattern, index)?;
        index += consumed;
        if pattern.get(index) == Some(&b'-') && pattern.get(index + 1) != Some(&b']') {
            let (end, end_consumed) = escaped_member(pattern, index + 1)?;
            matched |= start <= candidate && candidate <= end;
            index += 1 + end_consumed;
        } else {
            matched |= candidate == start;
        }
    }
    None
}

fn escaped_member(pattern: &[u8], index: usize) -> Option<(u8, usize)> {
    match pattern.get(index).copied()? {
        b'\\' => Some((*pattern.get(index + 1)?, 2)),
        member => Some((member, 1)),
    }
}

#[cfg(test)]
mod tests {
    use super::matches;

    #[test]
    fn supports_gitignore_wildcards_and_path_boundaries() {
        assert!(matches("*.rs", "lib.rs"));
        assert!(!matches("*.rs", "src/lib.rs"));
        assert!(matches("src/**/generated.rs", "src/a/b/generated.rs"));
        assert!(matches("src/**/generated.rs", "src/generated.rs"));
        assert!(!matches("src/*/generated.rs", "src/a/b/generated.rs"));
    }

    #[test]
    fn supports_character_classes_ranges_and_escaping() {
        assert!(matches("[a-c].rs", "b.rs"));
        assert!(!matches("[!a-c].rs", "b.rs"));
        assert!(matches("[!a-c].rs", "z.rs"));
        assert!(matches(r"file\[1\].rs", "file[1].rs"));
    }
}
