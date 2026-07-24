use super::ScanOptions;
use crate::walker::{ErrorPolicy, RootSymlinkPolicy};

impl ScanOptions {
    #[must_use]
    pub const fn with_max_depth(mut self, max_depth: Option<usize>) -> Self {
        self.walk.max_depth = max_depth;
        self
    }

    #[must_use]
    pub const fn with_min_depth(mut self, min_depth: usize) -> Self {
        self.walk.min_depth = min_depth;
        self
    }

    #[must_use]
    pub const fn with_max_open(mut self, max_open: usize) -> Self {
        self.walk.max_open = if max_open == 0 { 1 } else { max_open };
        self
    }

    #[must_use]
    pub const fn with_same_file_system(mut self, enabled: bool) -> Self {
        self.walk.same_file_system = enabled;
        self
    }

    #[must_use]
    pub const fn with_follow_links(mut self, enabled: bool) -> Self {
        self.walk.follow_links = enabled;
        self
    }

    #[must_use]
    pub const fn with_error_policy(mut self, policy: ErrorPolicy) -> Self {
        self.walk.error_policy = policy;
        self
    }

    #[must_use]
    pub const fn with_root_symlink_policy(mut self, policy: RootSymlinkPolicy) -> Self {
        self.walk.root_symlink_policy = policy;
        self
    }
}
