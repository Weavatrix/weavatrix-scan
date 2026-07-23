//! Deterministic, safe repository scanning for code-intelligence tools.
//!
//! `weavatrix-scan` never executes repository code, follows symlinks, or reads
//! files outside the canonical root.

mod config;
mod content;
mod error;
mod glob;
mod ignore;
mod path;
mod report;
mod scanner;

pub use config::{ScanOptions, StandardSkips};
pub use error::{Error, Result};
pub use ignore::IgnoreFile;
pub use report::{ScanReport, ScanWarning, ScannedFile, SkipKind, SkippedEntry};
pub use scanner::{Scanner, scan_repository};
