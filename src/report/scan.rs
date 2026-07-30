use super::{
    PathBuf, ScanCacheStats, ScanReport, ScanTermination, ScanWarning, SkipKind, SkippedEntry,
};

impl ScanReport {
    /// Computes the deterministic changed-file set from an older report.
    #[must_use]
    pub fn delta_from(&self, previous: &Self) -> crate::ScanDelta {
        crate::ScanDelta::between(previous, self)
    }

    pub(crate) fn new(root: PathBuf, record_skipped: bool) -> Self {
        Self {
            root,
            files: Vec::new(),
            skipped: Vec::new(),
            warnings: Vec::new(),
            ignore_sources: Vec::new(),
            revision: String::new(),
            complete: true,
            termination: None,
            portable: true,
            cache: ScanCacheStats::default(),
            record_skipped,
        }
    }

    pub(crate) fn skip(&mut self, relative: String, kind: SkipKind, detail: Option<String>) {
        if self.record_skipped {
            self.skipped.push(SkippedEntry {
                relative,
                kind,
                detail,
            });
        }
    }

    pub(crate) fn skip_borrowed(&mut self, relative: &str, kind: SkipKind, detail: Option<String>) {
        if self.record_skipped {
            self.skipped.push(SkippedEntry {
                relative: relative.to_owned(),
                kind,
                detail,
            });
        }
    }

    pub(crate) fn warn(&mut self, relative: Option<String>, message: impl Into<String>) {
        self.complete = false;
        self.warnings.push(ScanWarning {
            relative,
            message: message.into(),
        });
    }

    pub(crate) fn terminate(&mut self, reason: ScanTermination) {
        self.complete = false;
        self.termination.get_or_insert(reason);
    }

    pub(crate) fn finish_recording(&mut self) {
        self.record_skipped = true;
    }
}
