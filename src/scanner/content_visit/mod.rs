use super::Scanner;
use super::compact::{apply_total_bytes_limit, compact_revision, discover_compact, sort_evidence};
use super::entry::{process_entry_with, record_walk_error, walker_error_into_scan_error};
use crate::config::ScanOptions;
use crate::content::{ContentWorkerContext, VisitedFiles, visit_files};
use crate::content_visit::{
    ChangedContentVisitOutcome, ChangedContentVisitReport, ContentVisitControl, ContentVisitEvent,
    ContentVisitMode, ContentVisitReport,
};
use crate::error::{Error, Result};
use crate::ignore::RepositoryMatcher;
use crate::report::{
    CompactContentEvidence, CompactScanReport, CompactScannedFile, FileVersion, ScanReport,
};
use crate::runtime::ParallelRuntime;
use crate::scan_limits::ScanRuntime;
use crate::walker::{ErrorPolicy, Walker};
use std::collections::VecDeque;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, mpsc};

mod api;
mod changed;
mod discovery;
mod finish;
mod workers;

#[cfg(test)]
mod tests;

use changed::{PreparedDiscovery, visit_changed_content_plan, visit_content_direct};
use discovery::{
    collect_stream_outcomes, content_candidate, finish_stream_discovery, prepare_discovery,
    stream_discover_serial,
};
use finish::{FinishedContentVisit, finish_content_report};
use workers::run_workers;
