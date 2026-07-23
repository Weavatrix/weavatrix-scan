//! Deterministic, safe repository scanning for code-intelligence tools.
//!
//! `weavatrix-scan` never executes repository code or reads outside the
//! repository boundary. Symbolic links are skipped by default and guarded by
//! boundary/cycle checks when explicitly enabled.

mod config;
mod content;
mod error;
mod glob;
mod ignore;
mod parallel;
mod path;
#[cfg(feature = "serde")]
mod path_serde;
mod pool;
mod report;
mod scan_finalize;
mod scanner;
mod walk_iter;
mod walk_platform;
mod walk_types;
mod walk_visit;
mod walker;

pub use config::{ScanOptions, StandardSkips};
pub use error::{Error, Result};
pub use ignore::IgnoreFile;
pub use parallel::{ParallelWalkReport, ParallelWalker};
pub use report::{ScanReport, ScanWarning, ScannedFile, SkipKind, SkippedEntry};
pub use scanner::{Scanner, scan_repository};
pub use walker::{
    ErrorPolicy, WalkEntry, WalkError, WalkOperation, WalkOptions, WalkSkipReason, Walker,
};
