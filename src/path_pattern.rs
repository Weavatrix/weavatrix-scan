use std::fmt;

const MAX_ALTERNATIVE_EXPANSIONS: usize = 256;

/// A validated glob pattern for slash-separated repository paths.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathPattern {
    source: String,
}

impl PathPattern {
    /// Validates and retains a path pattern for repeated matching.
    ///
    /// # Errors
    ///
    /// Returns a positioned syntax error or rejects brace expansion that would
    /// create more than 256 alternatives.
    pub fn new(pattern: impl Into<String>) -> Result<Self, PathPatternError> {
        let source = pattern.into();
        validate(&source)?;
        Ok(Self { source })
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.source
    }

    #[must_use]
    pub fn is_match(&self, repository_path: &str) -> bool {
        crate::glob::matches(&self.source, repository_path)
    }
}

/// Stable category for a path-pattern validation failure.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathPatternErrorKind {
    Empty,
    DanglingEscape,
    UnclosedCharacterClass,
    InvalidCharacterRange,
    EmptyBraceAlternative,
    UnclosedBrace,
    UnexpectedClosingBrace,
    ExpansionLimitExceeded,
}

/// A path-pattern syntax error positioned at its first failing byte.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathPatternError {
    kind: PathPatternErrorKind,
    position: usize,
}

impl PathPatternError {
    const fn new(kind: PathPatternErrorKind, position: usize) -> Self {
        Self { kind, position }
    }

    #[must_use]
    pub const fn kind(&self) -> PathPatternErrorKind {
        self.kind
    }

    #[must_use]
    pub const fn position(&self) -> usize {
        self.position
    }
}

impl fmt::Display for PathPatternError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self.kind {
            PathPatternErrorKind::Empty => "pattern is empty",
            PathPatternErrorKind::DanglingEscape => "escape has no following byte",
            PathPatternErrorKind::UnclosedCharacterClass => "character class is not closed",
            PathPatternErrorKind::InvalidCharacterRange => "character range is descending",
            PathPatternErrorKind::EmptyBraceAlternative => "brace alternative is empty",
            PathPatternErrorKind::UnclosedBrace => "brace alternative is not closed",
            PathPatternErrorKind::UnexpectedClosingBrace => "closing brace has no opening brace",
            PathPatternErrorKind::ExpansionLimitExceeded => {
                "brace alternatives exceed the limit of 256"
            }
        };
        write!(
            formatter,
            "invalid path pattern at byte {}: {message}",
            self.position
        )
    }
}

impl std::error::Error for PathPatternError {}

#[derive(Debug)]
struct BraceState {
    start: usize,
    completed: usize,
    current: usize,
    has_token: bool,
}

impl BraceState {
    const fn new(start: usize) -> Self {
        Self {
            start,
            completed: 0,
            current: 1,
            has_token: false,
        }
    }

    fn finish_alternative(&mut self, position: usize) -> Result<(), PathPatternError> {
        if !self.has_token {
            return Err(PathPatternError::new(
                PathPatternErrorKind::EmptyBraceAlternative,
                position,
            ));
        }
        self.completed = bounded_add(self.completed, self.current, position)?;
        self.current = 1;
        self.has_token = false;
        Ok(())
    }

    fn finish(mut self, position: usize) -> Result<(usize, usize), PathPatternError> {
        self.finish_alternative(position)?;
        Ok((self.completed, self.start))
    }

    fn absorb(&mut self, count: usize, position: usize) -> Result<(), PathPatternError> {
        self.current = bounded_multiply(self.current, count, position)?;
        self.has_token = true;
        Ok(())
    }
}

fn validate(pattern: &str) -> Result<(), PathPatternError> {
    if pattern.is_empty() {
        return Err(PathPatternError::new(PathPatternErrorKind::Empty, 0));
    }
    let bytes = pattern.as_bytes();
    let mut braces = Vec::<BraceState>::new();
    let mut expansions = 1_usize;
    let mut index = 0_usize;
    while index < bytes.len() {
        match bytes[index] {
            b'\\' => {
                if index + 1 == bytes.len() {
                    return Err(PathPatternError::new(
                        PathPatternErrorKind::DanglingEscape,
                        index,
                    ));
                }
                mark_token(&mut braces);
                index += 2;
            }
            b'[' => {
                index = validate_character_class(bytes, index)?;
                mark_token(&mut braces);
            }
            b'{' => {
                braces.push(BraceState::new(index));
                index += 1;
            }
            b',' => {
                if let Some(state) = braces.last_mut() {
                    state.finish_alternative(index)?;
                }
                index += 1;
            }
            b'}' => {
                let Some(state) = braces.pop() else {
                    return Err(PathPatternError::new(
                        PathPatternErrorKind::UnexpectedClosingBrace,
                        index,
                    ));
                };
                let (count, start) = state.finish(index)?;
                if let Some(parent) = braces.last_mut() {
                    parent.absorb(count, start)?;
                } else {
                    expansions = bounded_multiply(expansions, count, start)?;
                }
                index += 1;
            }
            _ => {
                mark_token(&mut braces);
                index += 1;
            }
        }
    }
    if let Some(state) = braces.last() {
        return Err(PathPatternError::new(
            PathPatternErrorKind::UnclosedBrace,
            state.start,
        ));
    }
    Ok(())
}

fn validate_character_class(bytes: &[u8], start: usize) -> Result<usize, PathPatternError> {
    let mut index = start + 1;
    if matches!(bytes.get(index), Some(b'!' | b'^')) {
        index += 1;
    }
    let mut members = 0_usize;
    while index < bytes.len() {
        if bytes[index] == b']' && members > 0 {
            return Ok(index + 1);
        }
        let (member, next) = class_member(bytes, index)?;
        index = next;
        members += 1;
        if bytes.get(index) == Some(&b'-') && bytes.get(index + 1) != Some(&b']') {
            let range = index;
            let (end, next) = class_member(bytes, index + 1)?;
            if member > end {
                return Err(PathPatternError::new(
                    PathPatternErrorKind::InvalidCharacterRange,
                    range,
                ));
            }
            index = next;
        }
    }
    Err(PathPatternError::new(
        PathPatternErrorKind::UnclosedCharacterClass,
        start,
    ))
}

fn class_member(bytes: &[u8], index: usize) -> Result<(u8, usize), PathPatternError> {
    let Some(member) = bytes.get(index).copied() else {
        return Err(PathPatternError::new(
            PathPatternErrorKind::UnclosedCharacterClass,
            index.saturating_sub(1),
        ));
    };
    if member != b'\\' {
        return Ok((member, index + 1));
    }
    let Some(escaped) = bytes.get(index + 1).copied() else {
        return Err(PathPatternError::new(
            PathPatternErrorKind::DanglingEscape,
            index,
        ));
    };
    Ok((escaped, index + 2))
}

fn mark_token(braces: &mut [BraceState]) {
    if let Some(state) = braces.last_mut() {
        state.has_token = true;
    }
}

fn bounded_add(left: usize, right: usize, position: usize) -> Result<usize, PathPatternError> {
    let value = left.saturating_add(right);
    bounded(value, position)
}

fn bounded_multiply(left: usize, right: usize, position: usize) -> Result<usize, PathPatternError> {
    let value = left.saturating_mul(right);
    bounded(value, position)
}

fn bounded(value: usize, position: usize) -> Result<usize, PathPatternError> {
    if value <= MAX_ALTERNATIVE_EXPANSIONS {
        Ok(value)
    } else {
        Err(PathPatternError::new(
            PathPatternErrorKind::ExpansionLimitExceeded,
            position,
        ))
    }
}
