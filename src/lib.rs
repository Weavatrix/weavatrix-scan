//! Deterministic, safe repository scanning for code-intelligence tools.
//!
//! `weavatrix-scan` never executes repository code or reads outside the
//! repository boundary. Symbolic links are skipped by default and guarded by
//! boundary/cycle checks when explicitly enabled.
//!
//! Use [`ScanReport::to_portable`] to cross trust boundaries without exposing
//! host paths, and [`ScanReport::content_provider`] to reopen selected content
//! with snapshot verification.

mod cache;
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
mod multi_scanner;
mod parallel;
mod parallel_multi;
mod path;
#[cfg(feature = "serde")]
mod path_serde;
mod pool;
mod portable_report;
mod report;
mod scan_finalize;
mod scan_limits;
mod scan_match;
mod scan_stream;
mod scanner;
mod snapshot;
mod stateful_walk;
mod stdout;
mod walk_builder;
mod walk_iter;
mod walk_platform;
mod walk_types;
mod walk_visit;
mod walker;
mod watch;
#[cfg(feature = "notify")]
mod watch_notify;

pub use cache::{SCAN_CACHE_FORMAT_VERSION, ScanCache, ScanCacheEntry};
pub use config::{
    CacheValidationPolicy, EvidenceMode, IgnorePolicy, ScanLimits, ScanOptions, StandardSkips,
};
pub use control::CancellationToken;
pub use delta::{DeltaQuality, ModifiedFile, RenamedFile, ScanDelta};
pub use error::{Error, Result};
pub use file_types::NamedFileTypes;
pub use ignore::{IgnoreFile, RepositoryMatch, RepositoryMatcher};
pub use multi_scanner::{MultiScanReport, MultiScanner};
pub use parallel::{
    ParallelVisitReport, ParallelWalkIter, ParallelWalkReport, ParallelWalker, WalkControl,
    WalkEvent,
};
pub use parallel_multi::{ParallelMultiWalkReport, ParallelMultiWalker};
pub use portable_report::{
    PortableIgnoreSourceEvidence, PortableScanReport, PortableScanWarning, PortableScannedFile,
    PortableSkippedEntry,
};
pub use report::{
    FileIdentity, FileVersion, IgnoreSourceEvidence, IgnoreSourceKind, ScanCacheStats, ScanReport,
    ScanTermination, ScanWarning, ScannedFile, SkipKind, SkippedEntry,
};
pub use scan_stream::{ScanSink, ScanSinkControl, ScanStreamReport};
pub use scanner::{Scanner, scan_repository};
pub use snapshot::{SnapshotContent, SnapshotContentProvider, SnapshotEvidence, SnapshotReadError};
pub use stateful_walk::{StatefulWalkBuilder, StatefulWalkEntry, StatefulWalker};
pub use walk_builder::{MultiWalker, WalkBuilder};
pub use walker::{
    ErrorPolicy, RootSymlinkPolicy, WalkEntry, WalkError, WalkOperation, WalkOptions,
    WalkSkipReason, Walker,
};
pub use watch::{WatchEvent, WatchEventKind, WatchPlan, WatcherEventAdapter};
