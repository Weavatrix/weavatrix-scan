use std::time::Duration;

/// Hard bounds for hostile or unexpectedly large repository trees.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ScanLimits {
    pub max_entries: Option<u64>,
    pub max_total_bytes: Option<u64>,
    pub timeout: Option<Duration>,
}
