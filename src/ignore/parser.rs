use super::{IgnoreError, IgnoreRule, RuleAction, RuleMatcher, RuleScope, RuleSet, RuleTarget};
use std::io;
use std::path::Path;

pub(super) fn parse_file(
    path: &Path,
    text: &str,
    case_insensitive: bool,
    rules: &mut RuleSet,
    errors: &mut Vec<IgnoreError>,
) {
    parse_file_bytes(path, text.as_bytes(), case_insensitive, rules, errors);
}

pub(super) fn parse_file_bytes(
    path: &Path,
    bytes: &[u8],
    case_insensitive: bool,
    rules: &mut RuleSet,
    errors: &mut Vec<IgnoreError>,
) {
    let text = normalized_rule_text(bytes);
    for (index, raw) in text.trim_start_matches('\u{feff}').lines().enumerate() {
        match parse_rule(raw, case_insensitive) {
            Ok(Some(rule)) => rules.push(rule),
            Ok(None) => {}
            Err(message) => errors.push(IgnoreError {
                kind: io::ErrorKind::InvalidData,
                path: path.to_path_buf(),
                message: format!("line {}: {message}", index + 1),
            }),
        }
    }
}

fn normalized_rule_text(bytes: &[u8]) -> String {
    let mut normalized = String::with_capacity(bytes.len());
    let mut offset = 0;
    while offset < bytes.len() {
        match std::str::from_utf8(&bytes[offset..]) {
            Ok(valid) => {
                push_valid(&mut normalized, valid);
                break;
            }
            Err(error) => {
                let valid_end = offset + error.valid_up_to();
                if valid_end > offset {
                    let valid = std::str::from_utf8(&bytes[offset..valid_end])
                        .expect("UTF-8 validator supplied a valid prefix");
                    push_valid(&mut normalized, valid);
                }
                let invalid_len = error.error_len().unwrap_or(bytes.len() - valid_end);
                for byte in &bytes[valid_end..valid_end + invalid_len] {
                    use std::fmt::Write as _;
                    write!(normalized, "%{byte:02X}").expect("writing to String cannot fail");
                }
                offset = valid_end + invalid_len;
            }
        }
    }
    normalized
}

fn push_valid(output: &mut String, text: &str) {
    for character in text.chars() {
        if character == '%' {
            output.push_str("%25");
        } else {
            output.push(character);
        }
    }
}

pub(super) fn parse_rule(
    raw: &str,
    case_insensitive: bool,
) -> Result<Option<IgnoreRule>, &'static str> {
    let mut line = trim_unescaped_trailing_spaces(raw.trim_end_matches('\r')).to_owned();
    if line.is_empty() {
        return Ok(None);
    }
    let escaped_prefix = line.starts_with("\\#") || line.starts_with("\\!");
    if escaped_prefix {
        line.remove(0);
    } else if line.starts_with('#') {
        return Ok(None);
    }
    let negated = !escaped_prefix && line.starts_with('!');
    if negated {
        line.remove(0);
    }
    let escaped_directory = line.ends_with("\\/");
    let target = if line.ends_with('/') {
        line.pop();
        if escaped_directory {
            line.pop();
        }
        RuleTarget::Directory
    } else {
        RuleTarget::Any
    };
    let anchored = line.starts_with('/');
    if anchored {
        line.remove(0);
    }
    if line.is_empty() {
        return Ok(None);
    }
    validate_pattern(&line)?;
    if case_insensitive {
        line.make_ascii_lowercase();
    }
    let scope = if anchored {
        RuleScope::Anchored
    } else if line.contains('/') {
        RuleScope::Path
    } else {
        RuleScope::Anywhere
    };
    Ok(Some(IgnoreRule {
        matcher: RuleMatcher::new(&line, case_insensitive),
        pattern: line,
        action: if negated {
            RuleAction::Include
        } else {
            RuleAction::Ignore
        },
        target,
        scope,
    }))
}

fn validate_pattern(pattern: &str) -> Result<(), &'static str> {
    let mut escaped = false;
    let mut bracket = false;
    let mut brace = 0_i32;
    for byte in pattern.bytes() {
        if escaped {
            escaped = false;
        } else {
            match byte {
                b'\\' => escaped = true,
                b'[' if !bracket => bracket = true,
                b']' if bracket => bracket = false,
                b'{' if !bracket => brace += 1,
                b'}' if !bracket => {
                    brace -= 1;
                    if brace < 0 {
                        return Err("unmatched closing brace");
                    }
                }
                _ => {}
            }
        }
    }
    if brace != 0 {
        Err("unclosed alternation")
    } else {
        Ok(())
    }
}

fn trim_unescaped_trailing_spaces(mut line: &str) -> &str {
    while let Some(without_space) = line.strip_suffix(' ') {
        let slash_count = without_space
            .bytes()
            .rev()
            .take_while(|byte| *byte == b'\\')
            .count();
        if slash_count % 2 == 1 {
            break;
        }
        line = without_space;
    }
    line
}
