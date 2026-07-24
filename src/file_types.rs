use crate::glob;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// Named, reusable groups of file-name or repository-relative glob patterns.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NamedFileTypes {
    definitions: BTreeMap<String, BTreeSet<FileTypePattern>>,
    selections: Vec<FileTypeSelection>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum FileTypePattern {
    Extension(String),
    Glob(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum FileTypeSelection {
    Include(String),
    Exclude(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FileTypeMatch {
    Include,
    Exclude,
    None,
}

impl NamedFileTypes {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a catalog containing common source, markup, data, and build
    /// file types. No type is selected until [`Self::select`] is called.
    #[must_use]
    pub fn defaults() -> Self {
        Self::new().with_defaults()
    }

    /// Adds built-in definitions without replacing caller-defined names.
    #[must_use]
    pub fn with_defaults(mut self) -> Self {
        for &(names, patterns) in DEFAULT_FILE_TYPES {
            let parsed = patterns
                .iter()
                .map(|pattern| pattern_from_glob(pattern))
                .collect::<BTreeSet<_>>();
            for name in names {
                self.definitions
                    .entry((*name).to_owned())
                    .or_insert_with(|| parsed.clone());
            }
        }
        self
    }

    /// Adds or replaces a named extension group.
    ///
    /// Extensions retain a specialized allocation-free matching path.
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
                    FileTypePattern::Extension(normalized_extension(extension.as_ref()))
                })
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
                .map(|pattern| pattern_from_glob(pattern.as_ref()))
                .collect(),
        );
        self
    }

    /// Adds or replaces a type composed from existing definitions.
    ///
    /// Unknown component names contribute no patterns.
    #[must_use]
    pub fn with_composed_type<I, S>(mut self, name: impl Into<String>, components: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut patterns = BTreeSet::new();
        for component in components {
            if let Some(component) = self.definitions.get(component.as_ref()) {
                patterns.extend(component.iter().cloned());
            }
        }
        self.definitions.insert(name.into(), patterns);
        self
    }

    /// Replaces current selections with included named groups.
    ///
    /// Unknown names simply match no files.
    #[must_use]
    pub fn select<I, S>(mut self, names: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        self.selections = names
            .into_iter()
            .map(|name| FileTypeSelection::Include(name.as_ref().to_owned()))
            .collect();
        self
    }

    /// Appends excluded named groups. Later matching selections win.
    #[must_use]
    pub fn negate<I, S>(mut self, names: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        self.selections.extend(
            names
                .into_iter()
                .map(|name| FileTypeSelection::Exclude(name.as_ref().to_owned())),
        );
        self
    }

    #[must_use]
    pub fn is_active(&self) -> bool {
        !self.selections.is_empty()
    }

    #[must_use]
    pub fn contains(&self, name: &str) -> bool {
        self.definitions.contains_key(name)
    }

    pub(crate) fn has_includes(&self) -> bool {
        self.selections
            .iter()
            .any(|selection| matches!(selection, FileTypeSelection::Include(_)))
    }

    pub(crate) fn matched(&self, path: &Path, relative: &str) -> FileTypeMatch {
        let mut matched = FileTypeMatch::None;
        for selection in &self.selections {
            let (name, decision) = match selection {
                FileTypeSelection::Include(name) => (name, FileTypeMatch::Include),
                FileTypeSelection::Exclude(name) => (name, FileTypeMatch::Exclude),
            };
            if self
                .definitions
                .get(name)
                .is_some_and(|patterns| matches_any(patterns, path, relative))
            {
                matched = decision;
            }
        }
        matched
    }
}

fn matches_any(patterns: &BTreeSet<FileTypePattern>, path: &Path, relative: &str) -> bool {
    let file_name = path.file_name().and_then(|value| value.to_str());
    patterns.iter().any(|pattern| match pattern {
        FileTypePattern::Extension(expected) => path
            .extension()
            .and_then(|value| value.to_str())
            .is_some_and(|actual| actual == expected || actual.eq_ignore_ascii_case(expected)),
        FileTypePattern::Glob(pattern) if pattern.contains('/') => glob::matches(pattern, relative),
        FileTypePattern::Glob(pattern) => {
            file_name.is_some_and(|file_name| glob::matches(pattern, file_name))
        }
    })
}

fn pattern_from_glob(pattern: &str) -> FileTypePattern {
    let normalized = pattern.replace('\\', "/");
    normalized
        .strip_prefix("*.")
        .filter(|extension| {
            !extension.contains('.')
                && !extension
                    .bytes()
                    .any(|byte| matches!(byte, b'*' | b'?' | b'[' | b'{' | b'/'))
        })
        .map_or_else(
            || FileTypePattern::Glob(normalized.clone()),
            |extension| FileTypePattern::Extension(normalized_extension(extension)),
        )
}

fn normalized_extension(extension: &str) -> String {
    extension.trim_start_matches('.').to_ascii_lowercase()
}

#[rustfmt::skip]
const DEFAULT_FILE_TYPES: &[(&[&str], &[&str])] = &[
    (&["c"], &["*.c", "*.h"]),
    (&["cmake"], &["*.cmake", "CMakeLists.txt"]),
    (&["cpp", "cxx"], &["*.cc", "*.cpp", "*.cxx", "*.hh", "*.hpp", "*.hxx"]),
    (&["csharp", "cs"], &["*.cs", "*.csproj"]),
    (&["css"], &["*.css", "*.scss", "*.sass", "*.less"]),
    (&["dart"], &["*.dart"]),
    (&["docker", "container"], &["Dockerfile", "Dockerfile.*", "Containerfile", "Containerfile.*"]),
    (&["elixir"], &["*.ex", "*.exs", "*.heex"]),
    (&["erlang"], &["*.erl", "*.hrl"]),
    (&["go"], &["*.go"]),
    (&["graphql"], &["*.graphql", "*.graphqls"]),
    (&["haskell"], &["*.hs", "*.lhs"]),
    (&["html"], &["*.html", "*.htm", "*.ejs"]),
    (&["java"], &["*.java", "*.jsp"]),
    (&["javascript", "js"], &["*.js", "*.jsx", "*.mjs", "*.cjs"]),
    (&["json"], &["*.json", "*.jsonl", "*.sarif"]),
    (&["julia"], &["*.jl"]),
    (&["kotlin"], &["*.kt", "*.kts"]),
    (&["lua"], &["*.lua"]),
    (&["make"], &["Makefile", "makefile", "GNUmakefile", "*.mk"]),
    (&["markdown", "md"], &["*.md", "*.markdown", "*.mdx"]),
    (&["php"], &["*.php", "*.php3", "*.php4", "*.php5", "*.phtml"]),
    (&["protobuf", "proto"], &["*.proto"]),
    (&["python", "py"], &["*.py", "*.pyi", "*.pyw"]),
    (&["ruby"], &["*.rb", "Gemfile", "Rakefile"]),
    (&["rust", "rs"], &["*.rs"]),
    (&["scala"], &["*.scala", "*.sbt"]),
    (&["shell", "sh"], &["*.sh", "*.bash", "*.zsh", "*.fish"]),
    (&["sql"], &["*.sql"]),
    (&["svelte"], &["*.svelte"]),
    (&["swift"], &["*.swift"]),
    (&["terraform", "hcl"], &["*.tf", "*.tfvars", "*.hcl"]),
    (&["toml"], &["*.toml", "Cargo.lock"]),
    (&["typescript", "ts"], &["*.ts", "*.tsx", "*.mts", "*.cts"]),
    (&["vue"], &["*.vue"]),
    (&["xml"], &["*.xml", "*.xsd", "*.xsl", "*.xslt"]),
    (&["yaml", "yml"], &["*.yaml", "*.yml"]),
    (&["zig"], &["*.zig", "*.zon"]),
];
