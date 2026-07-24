//! Deterministic, safe repository scanning for code-intelligence tools.
//!
//! `weavatrix-scan` never executes repository code or reads outside the
//! repository boundary. Symbolic links are skipped by default and guarded by
//! boundary/cycle checks when explicitly enabled.

mod config;
mod content;
mod control;
mod delta;
mod error;
mod file_types;
mod file_version;
mod glob;
mod hash;
mod hidden;
mod ignore;
mod parallel;
mod path;
#[cfg(feature = "serde")]
mod path_serde;
mod pool;
mod report;
mod scan_finalize;
mod scan_limits;
mod scan_match;
mod scanner;
mod walk_builder;
mod walk_iter;
mod walk_platform;
mod walk_types;
mod walk_visit;
mod walker;

pub use config::{EvidenceMode, IgnorePolicy, ScanLimits, ScanOptions, StandardSkips};
pub use control::CancellationToken;
pub use delta::{DeltaQuality, ModifiedFile, RenamedFile, ScanDelta};
pub use error::{Error, Result};
pub use file_types::NamedFileTypes;
pub use ignore::{IgnoreFile, RepositoryMatch, RepositoryMatcher};
pub use parallel::{
    ParallelVisitReport, ParallelWalkReport, ParallelWalker, WalkControl, WalkEvent,
};
pub use report::{
    FileIdentity, FileVersion, IgnoreSourceEvidence, IgnoreSourceKind, ScanCacheStats, ScanReport,
    ScanTermination, ScanWarning, ScannedFile, SkipKind, SkippedEntry,
};
pub use scanner::{Scanner, scan_repository};
pub use walk_builder::{MultiWalker, WalkBuilder};
pub use walker::{
    ErrorPolicy, WalkEntry, WalkError, WalkOperation, WalkOptions, WalkSkipReason, Walker,
};
