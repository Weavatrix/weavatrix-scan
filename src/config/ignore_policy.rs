use std::path::PathBuf;

/// Controls every supported source of repository selection rules.
///
/// The default is repository-local and reproducible. Use [`Self::git_compatible`]
/// when matching the current machine's Git configuration is more important
/// than producing a portable manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(clippy::struct_excessive_bools)]
pub struct IgnorePolicy {
    pub parent_rules: bool,
    pub git_ignore: bool,
    pub dot_ignore: bool,
    pub custom_ignore: bool,
    pub git_exclude: bool,
    pub git_global: bool,
    /// Apply Git-specific sources only when a repository root is present.
    pub require_git: bool,
    pub explicit_files: Vec<PathBuf>,
}

impl Default for IgnorePolicy {
    fn default() -> Self {
        Self::repository()
    }
}

impl IgnorePolicy {
    #[must_use]
    pub const fn repository() -> Self {
        Self {
            parent_rules: false,
            git_ignore: true,
            dot_ignore: true,
            custom_ignore: true,
            git_exclude: false,
            git_global: false,
            require_git: false,
            explicit_files: Vec::new(),
        }
    }

    #[must_use]
    pub const fn git_compatible() -> Self {
        Self {
            parent_rules: true,
            git_ignore: true,
            dot_ignore: true,
            custom_ignore: true,
            git_exclude: true,
            git_global: true,
            require_git: true,
            explicit_files: Vec::new(),
        }
    }

    #[must_use]
    pub const fn with_parent_rules(mut self, enabled: bool) -> Self {
        self.parent_rules = enabled;
        self
    }

    #[must_use]
    pub const fn with_git_ignore(mut self, enabled: bool) -> Self {
        self.git_ignore = enabled;
        self
    }

    #[must_use]
    pub const fn with_dot_ignore(mut self, enabled: bool) -> Self {
        self.dot_ignore = enabled;
        self
    }

    #[must_use]
    pub const fn with_custom_ignore(mut self, enabled: bool) -> Self {
        self.custom_ignore = enabled;
        self
    }

    #[must_use]
    pub const fn with_git_exclude(mut self, enabled: bool) -> Self {
        self.git_exclude = enabled;
        self
    }

    #[must_use]
    pub const fn with_git_global(mut self, enabled: bool) -> Self {
        self.git_global = enabled;
        self
    }

    #[must_use]
    pub const fn with_require_git(mut self, enabled: bool) -> Self {
        self.require_git = enabled;
        self
    }

    #[must_use]
    pub fn with_explicit_file(mut self, path: impl Into<PathBuf>) -> Self {
        self.explicit_files.push(path.into());
        self
    }
}
