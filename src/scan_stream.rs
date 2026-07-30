use crate::report::{
    IgnoreSourceEvidence, ScanCacheStats, ScanTermination, ScanWarning, ScannedFile, SkippedEntry,
};
use std::path::PathBuf;

/// Controls deterministic file emission from [`crate::Scanner::scan_into`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScanSinkControl {
    Continue,
    Stop,
}

/// Synchronous consumer for selected scan entries.
///
/// The scanner does not request the next item until this method returns, which
/// gives consumers natural backpressure without an unbounded channel.
pub trait ScanSink {
    fn on_file(&mut self, file: &ScannedFile) -> ScanSinkControl;
}

impl<F> ScanSink for F
where
    F: FnMut(&ScannedFile) -> ScanSinkControl,
{
    fn on_file(&mut self, file: &ScannedFile) -> ScanSinkControl {
        self(file)
    }
}

/// Summary of deterministic, backpressured scan emission.
///
/// Selected file records are owned by the sink and are intentionally not
/// retained here.
#[derive(Debug)]
pub struct ScanStreamReport {
    pub root: PathBuf,
    pub selected: u64,
    pub emitted: u64,
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
