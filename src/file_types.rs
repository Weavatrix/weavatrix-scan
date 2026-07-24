use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// Named, reusable groups of file extensions.
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
                .map(|extension| {
                    extension
                        .as_ref()
                        .trim_start_matches('.')
                        .to_ascii_lowercase()
                })
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

    pub(crate) fn accepts(&self, path: &Path) -> bool {
        let Some(extension) = path.extension().and_then(|value| value.to_str()) else {
            return false;
        };
        let extension = extension.to_ascii_lowercase();
        self.selected.iter().any(|name| {
            self.definitions
                .get(name)
                .is_some_and(|extensions| extensions.contains(&extension))
        })
    }
}
