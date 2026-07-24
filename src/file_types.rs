use crate::glob;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// Named, reusable groups of file-name or repository-relative glob patterns.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NamedFileTypes {
    definitions: BTreeMap<String, BTreeSet<String>>,
    selected: BTreeSet<String>,
}

impl NamedFileTypes {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds or replaces a named extension group.
    ///
    /// Extensions are stored as file-name globs, so this remains compatible
    /// with definitions created before arbitrary glob support was added.
    #[must_use]
    pub fn with_type<I, S>(mut self, name: impl Into<String>, extensions: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        self.definitions.insert(
            name.into(),
            extensions
                .into_iter()
                .map(|extension| format!("*.{}", normalized_extension(extension.as_ref())))
                .collect(),
        );
        self
    }

    /// Adds or replaces a named group of arbitrary glob patterns.
    ///
    /// Patterns containing `/` match normalized repository-relative paths.
    /// Other patterns match only the native file name. Matching is
    /// case-sensitive; callers can include explicit variants when needed.
    #[must_use]
    pub fn with_globs<I, S>(mut self, name: impl Into<String>, patterns: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        self.definitions.insert(
            name.into(),
            patterns
                .into_iter()
                .map(|pattern| pattern.as_ref().replace('\\', "/"))
                .collect(),
        );
        self
    }

    /// Selects named groups. Unknown names simply match no files.
    #[must_use]
    pub fn select<I, S>(mut self, names: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        self.selected = names
            .into_iter()
            .map(|name| name.as_ref().to_owned())
            .collect();
        self
    }

    #[must_use]
    pub fn is_active(&self) -> bool {
        !self.selected.is_empty()
    }

    pub(crate) fn accepts(&self, path: &Path, relative: &str) -> bool {
        let file_name = path.file_name().and_then(|value| value.to_str());
        self.selected.iter().any(|name| {
            self.definitions.get(name).is_some_and(|patterns| {
                patterns.iter().any(|pattern| {
                    if pattern.contains('/') {
                        glob::matches(pattern, relative)
                    } else {
                        file_name.is_some_and(|file_name| glob::matches(pattern, file_name))
                    }
                })
            })
        })
    }
}

fn normalized_extension(extension: &str) -> String {
    extension.trim_start_matches('.').to_ascii_lowercase()
}
