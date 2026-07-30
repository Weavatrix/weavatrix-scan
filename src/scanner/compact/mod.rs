use super::entry::{process_entry_with, record_walk_error, walker_error_into_scan_error};
use crate::config::{EvidenceMode, ScanOptions};
use crate::content::inspect_files;
use crate::error::{Error, Result};
use crate::hash::FingerprintHasher;
use crate::ignore::RepositoryMatcher;
use crate::parallel::dynamic;
use crate::report::{
    CompactContentEvidence, CompactScanReport, CompactScannedFile, ScanReport, ScannedFile,
    SkipKind, SkippedEntry,
};
use crate::runtime::ParallelRuntime;
use crate::scan_limits::ScanRuntime;
use crate::walker::{ErrorPolicy, Walker};
use std::path::Path;
use std::sync::{Arc, Mutex};

mod discovery;
mod inspection;

pub use discovery::scan_repository_compact;
pub(super) use discovery::{discover_compact, scan_repository_compact_with_runtime};
pub(super) use inspection::{apply_total_bytes_limit, compact_revision, sort_evidence};
use inspection::{compact_file, inspect_compact};
