use crate::config::{ContentValidationPolicy, ScanOptions};
use crate::content::bounded::{BoundedReadLimits, ReadControl};
use crate::content_visit::{
    ContentFile, ContentFileStatus, ContentVisitControl, ContentVisitEvent,
};
use crate::file_version::{reusable, snapshot};
use crate::hash::{ContentFingerprint, FingerprintHasher};
use crate::report::ScannedFile;
use std::fs::File;
use std::io;
use std::path::Path;
use std::time::Instant;

use super::ContentWorkerContext;

fn read_control(options: &ScanOptions, started: Instant) -> ReadControl<'_> {
    ReadControl {
        cancellation: options.cancellation.as_ref(),
        deadline: options.limits.timeout.map(|timeout| started + timeout),
    }
}

fn read_limits(expected_bytes: u64, options: &ScanOptions) -> BoundedReadLimits {
    BoundedReadLimits {
        expected_bytes,
        max_content_bytes: options.max_file_bytes,
    }
}

mod file;
mod support;
mod visitor;

pub(super) use file::inspect;
pub(super) use support::validate_cached;
pub(super) use visitor::inspect_with_visitor;

pub(super) enum Inspection {
    Selected(ScannedFile),
    Binary(String),
    Concurrent(String),
}

pub(super) enum CachedValidation {
    Match,
    Changed,
    Concurrent,
}

pub(super) struct VisitedInspection {
    pub status: Option<VisitedStatus>,
    pub opened: u64,
    pub chunks: u64,
    pub bytes_read: u64,
    pub bytes_emitted: u64,
    pub consumer_skipped: bool,
    pub visitor_quit: bool,
}

pub(super) enum VisitedStatus {
    Selected,
    Binary,
    Concurrent,
}
