use super::{CompactScanReport, CompactScannedFile, FileVersion, PathBuf, ScanReport, ScannedFile};

impl CompactScanReport {
    /// Materializes an absolute path for one compact entry.
    #[must_use]
    pub fn absolute_path(&self, file: &CompactScannedFile) -> PathBuf {
        self.root.join(file.relative.as_ref())
    }

    /// Materializes the compatibility report without reading file contents
    /// again.
    #[must_use]
    pub fn into_scan_report(self) -> ScanReport {
        let root = self.root;
        let files = self
            .files
            .into_iter()
            .map(|file| {
                let content = file.content.map(|content| *content);
                ScannedFile {
                    absolute: root.join(file.relative.as_ref()),
                    relative: file.relative.into(),
                    bytes: file.bytes,
                    content_hash: content
                        .as_ref()
                        .and_then(|value| value.content_hash.as_deref())
                        .map(str::to_owned),
                    content_fingerprint: content
                        .as_ref()
                        .and_then(|value| value.content_fingerprint.as_deref())
                        .map(str::to_owned),
                    version: content
                        .as_ref()
                        .map_or_else(FileVersion::default, |value| value.version),
                    binary_checked: content.is_some_and(|value| value.binary_checked),
                }
            })
            .collect();
        ScanReport {
            root,
            files,
            skipped: self.skipped,
            warnings: self.warnings,
            ignore_sources: self.ignore_sources,
            revision: self.revision,
            complete: self.complete,
            termination: self.termination,
            portable: self.portable,
            cache: self.cache,
            record_skipped: true,
        }
    }
}
