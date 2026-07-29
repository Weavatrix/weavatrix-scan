use crate::report::{
    CompactScanReport, CompactScannedFile, IgnoreSourceEvidence, ScanCacheStats, ScanTermination,
    ScanWarning, SkippedEntry,
};
use std::path::{Path, PathBuf};

/// Controls whether a content visit retains selected-file evidence internally.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentVisitMode {
    /// Retain compact selected-file evidence and compute a deterministic
    /// revision.
    Revision,
    /// Emit bytes and counters without retaining selected-file evidence or
    /// computing a revision.
    Streaming,
}

/// Controls delivery from [`crate::Scanner::visit_content`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentVisitControl {
    /// Continue delivering the current file.
    Continue,
    /// Stop delivering chunks for this file while allowing required scanner
    /// evidence work to finish.
    SkipFile,
    /// Cooperatively stop every content worker.
    Quit,
}

/// Stable identity and discovery evidence for one selected file.
#[derive(Debug, Clone, Copy)]
pub struct ContentFile<'a> {
    /// Root insertion index. A single-root [`crate::Scanner`] always uses zero.
    pub root_index: usize,
    /// Monotonic work sequence within this visit. Sort by `root_index` and
    /// `relative` when results must be deterministic across runs.
    pub sequence: u64,
    pub root: &'a Path,
    pub absolute: &'a Path,
    pub relative: &'a str,
    pub bytes: u64,
}

/// Result of verifying the file around its single content read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentFileStatus {
    Selected,
    Binary,
    Changed,
}

/// Events emitted from one bounded content worker.
#[derive(Debug)]
pub enum ContentVisitEvent<'a> {
    /// The selected file was opened and its discovery evidence was verified.
    FileStart {
        worker_index: usize,
        file: ContentFile<'a>,
    },
    /// A borrowed chunk from the scanner's one content read.
    Chunk {
        worker_index: usize,
        file: ContentFile<'a>,
        offset: u64,
        bytes: &'a [u8],
    },
    /// The file read and post-read verification finished.
    FileEnd {
        worker_index: usize,
        file: ContentFile<'a>,
        status: ContentFileStatus,
        bytes_read: u64,
        content_hash: Option<&'a str>,
        consumer_skipped: bool,
    },
}

/// Summary of a parallel, one-pass selected-content visit.
#[derive(Debug)]
pub struct ContentVisitReport {
    pub mode: ContentVisitMode,
    pub root: PathBuf,
    /// Deterministic selected-file manifest retained in revision mode.
    ///
    /// Every entry carries the content hash and file-version evidence produced
    /// by the same read that emitted its bytes to the visitor. Streaming mode
    /// leaves this empty.
    pub files: Vec<CompactScannedFile>,
    /// Candidates selected by traversal and ignore rules before content checks.
    pub discovered: u64,
    /// Files that completed content checks and remain selected.
    pub completed: u64,
    pub opened: u64,
    pub chunks: u64,
    pub bytes_read: u64,
    pub bytes_emitted: u64,
    pub consumer_skipped: u64,
    pub stopped: bool,
    pub skipped: Vec<SkippedEntry>,
    pub warnings: Vec<ScanWarning>,
    pub ignore_sources: Vec<IgnoreSourceEvidence>,
    pub revision: String,
    pub complete: bool,
    pub termination: Option<ScanTermination>,
    pub portable: bool,
    pub cache: ScanCacheStats,
}

impl ContentVisitReport {
    /// Converts a revision-mode visit into the compact scanner manifest used
    /// by incremental consumers.
    ///
    /// Streaming visits deliberately do not retain file evidence and therefore
    /// cannot be converted.
    ///
    /// # Errors
    ///
    /// Returns an error when called for a streaming visit.
    pub fn into_compact_scan_report(self) -> Result<CompactScanReport, &'static str> {
        if self.mode != ContentVisitMode::Revision {
            return Err("streaming content visits do not retain a file manifest");
        }
        Ok(CompactScanReport {
            root: self.root,
            files: self.files,
            skipped: self.skipped,
            warnings: self.warnings,
            ignore_sources: self.ignore_sources,
            revision: self.revision,
            complete: self.complete,
            termination: self.termination,
            portable: self.portable,
            cache: self.cache,
        })
    }
}

/// Ordered summaries from a multi-root content visit.
#[derive(Debug)]
pub struct MultiContentVisitReport {
    /// Reports remain in the same order as roots were added.
    pub reports: Vec<ContentVisitReport>,
}

impl MultiContentVisitReport {
    #[must_use]
    pub const fn len(&self) -> usize {
        self.reports.len()
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.reports.is_empty()
    }
}

/// Content and removals produced by a safe file-only watcher plan.
#[derive(Debug)]
pub struct ChangedContentVisitReport {
    /// Evidence for the changed files that still exist and remain selected.
    ///
    /// Its revision describes this changed-file subset, not the complete
    /// repository manifest.
    pub content: ContentVisitReport,
    /// Stable normalized paths that disappeared from the repository.
    pub removed: Vec<String>,
}

/// Result of attempting a traversal-free watcher content visit.
#[derive(Debug)]
pub enum ChangedContentVisitOutcome {
    /// Only changed file paths were matched, opened, and visited.
    Visited(Box<ChangedContentVisitReport>),
    /// The plan can affect directory structure or selection and therefore
    /// requires the caller to perform a complete scan.
    FullRescanRequired,
}
