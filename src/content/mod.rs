use crate::cache::{ScanCache, ScanCacheEntry};
use crate::config::{CacheValidationPolicy, EvidenceMode, ScanOptions};
use crate::content_visit::{ContentVisitControl, ContentVisitEvent, ContentVisitMode};
use crate::error::{Error, Result};
use crate::file_version::reusable;
use crate::report::{
    CompactContentEvidence, CompactScannedFile, ScanCacheStats, ScanTermination, ScanWarning,
    ScannedFile, SkipKind, SkippedEntry,
};
use crate::walker::ErrorPolicy;
use inspect::{Inspection, VisitedStatus};
use std::collections::HashMap;
use std::io;
use std::path::Path;
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::Instant;

mod batch;
mod inspect;
mod support;
mod visit;

pub(crate) use batch::inspect_files;
use support::{
    InspectionStop, apply_cached, record_binary_skip, record_concurrent_modification,
    record_io_error, record_limit_skip, reusable_candidate,
};
pub(crate) use visit::visit_files;

#[derive(Clone, Copy)]
pub(crate) struct ContentWorkerContext<'a> {
    pub(crate) root: &'a Path,
    pub(crate) root_index: usize,
    pub(crate) worker_index: usize,
    pub(crate) mode: ContentVisitMode,
}

pub(crate) struct InspectedFiles {
    pub(crate) files: Vec<ScannedFile>,
    pub(crate) skipped: Vec<SkippedEntry>,
    pub(crate) warnings: Vec<ScanWarning>,
    pub(crate) termination: Option<ScanTermination>,
    pub(crate) cache: ScanCacheStats,
}

pub(crate) struct VisitedFiles {
    pub(crate) files: Vec<(u64, CompactScannedFile)>,
    pub(crate) evidence: InspectedFiles,
    pub(crate) opened: u64,
    pub(crate) chunks: u64,
    pub(crate) bytes_read: u64,
    pub(crate) bytes_emitted: u64,
    pub(crate) consumer_skipped: u64,
    pub(crate) completed: u64,
    pub(crate) visitor_quit: bool,
}

impl VisitedFiles {
    pub(crate) fn empty(capacity: usize) -> Self {
        Self {
            files: Vec::with_capacity(capacity),
            evidence: InspectedFiles {
                files: Vec::new(),
                skipped: Vec::new(),
                warnings: Vec::new(),
                termination: None,
                cache: ScanCacheStats::default(),
            },
            opened: 0,
            chunks: 0,
            bytes_read: 0,
            bytes_emitted: 0,
            consumer_skipped: 0,
            completed: 0,
            visitor_quit: false,
        }
    }

    pub(crate) fn merge(&mut self, other: Self) {
        self.files.extend(other.files);
        self.evidence.skipped.extend(other.evidence.skipped);
        self.evidence.warnings.extend(other.evidence.warnings);
        self.evidence.termination = self.evidence.termination.or(other.evidence.termination);
        self.evidence.cache.reused_hashes = self
            .evidence
            .cache
            .reused_hashes
            .saturating_add(other.evidence.cache.reused_hashes);
        self.evidence.cache.content_reads = self
            .evidence
            .cache
            .content_reads
            .saturating_add(other.evidence.cache.content_reads);
        self.evidence.cache.fingerprint_reads = self
            .evidence
            .cache
            .fingerprint_reads
            .saturating_add(other.evidence.cache.fingerprint_reads);
        self.opened = self.opened.saturating_add(other.opened);
        self.chunks = self.chunks.saturating_add(other.chunks);
        self.bytes_read = self.bytes_read.saturating_add(other.bytes_read);
        self.bytes_emitted = self.bytes_emitted.saturating_add(other.bytes_emitted);
        self.consumer_skipped = self.consumer_skipped.saturating_add(other.consumer_skipped);
        self.completed = self.completed.saturating_add(other.completed);
        self.visitor_quit |= other.visitor_quit;
    }
}

#[cfg(test)]
mod tests;
