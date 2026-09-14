use crate::report::ScanReport;

/// Why a watch-plan apply reused retained evidence or rebuilt the tree.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WatchUpdateReason {
    /// Retained evidence was merged with the changed paths only.
    Incremental,
    /// A complete scan replaced the previous snapshot.
    FullRescan(FullRescanReason),
}

/// Typed cause of a complete rescan after a watch-plan apply.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FullRescanReason {
    /// Selection or content policy no longer matches the stored descriptor.
    PolicyChanged,
    /// An ignore input was added, changed, or deleted in the visited scope.
    IgnoreInputChanged,
    /// A directory, root, or other unsafe structural event was observed.
    StructuralChange,
    /// The previous snapshot was incomplete, terminated, or entry-bounded.
    IncompletePreviousState,
}

impl WatchUpdateReason {
    /// Stable label for logs, Node bindings, and benchmarks.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Incremental => "Incremental",
            Self::FullRescan(FullRescanReason::PolicyChanged) => "FullRescan:PolicyChanged",
            Self::FullRescan(FullRescanReason::IgnoreInputChanged) => {
                "FullRescan:IgnoreInputChanged"
            }
            Self::FullRescan(FullRescanReason::StructuralChange) => "FullRescan:StructuralChange",
            Self::FullRescan(FullRescanReason::IncompletePreviousState) => {
                "FullRescan:IncompletePreviousState"
            }
        }
    }
}

impl core::fmt::Display for WatchUpdateReason {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Report produced by applying a [`crate::WatchPlan`], with the apply path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WatchUpdate {
    /// Manifest after the apply.
    pub report: ScanReport,
    /// Whether the fast path ran or a complete scan replaced it.
    pub reason: WatchUpdateReason,
}
