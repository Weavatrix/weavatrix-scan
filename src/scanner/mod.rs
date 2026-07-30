use crate::cache::ScanCache;
use crate::config::{EvidenceMode, ScanOptions};
use crate::content::inspect_files;
use crate::error::{Error, Result};
use crate::ignore::RepositoryMatcher;
use crate::parallel::dynamic;
use crate::report::{ScanReport, ScannedFile};
use crate::runtime::ParallelRuntime;
use crate::scan_finalize::finalize_report;
use crate::scan_limits::{ScanRuntime, apply_total_bytes_limit};
use crate::walker::{ErrorPolicy, Walker};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

mod api;
mod batch;
mod compact;
mod content_visit;
mod discovery;
mod entry;
mod stream;
mod watch_update;

use entry::{process_entry, record_walk_error, walker_error_into_scan_error};

pub use compact::scan_repository_compact;

pub struct Scanner {
    root: PathBuf,
    options: ScanOptions,
    runtime: ParallelRuntime,
}

pub use discovery::scan_repository;
pub(crate) use discovery::scan_repository_with_runtime;
