use super::{
    CompactContentEvidence, CompactScanReport, CompactScannedFile, FileVersion, PathBuf,
    ScanReport, ScannedFile,
};

impl ScanReport {
    /// Copies selected files and scan state into a root-shared compact manifest.
    ///
    /// Unlike [`Self::to_cache`], this keeps every selected file plus
    /// completeness, termination, and ignore evidence.
    #[must_use]
    pub fn to_compact(&self) -> CompactScanReport {
        CompactScanReport {
            root: self.root.clone(),
            files: self.files.iter().map(CompactScannedFile::from).collect(),
            skipped: self.skipped.clone(),
            warnings: self.warnings.clone(),
            ignore_sources: self.ignore_sources.clone(),
            revision: self.revision.clone(),
            descriptor: self.descriptor.clone(),
            complete: self.complete,
            termination: self.termination,
            portable: self.portable,
            cache: self.cache,
        }
    }
}

impl From<&ScannedFile> for CompactScannedFile {
    fn from(file: &ScannedFile) -> Self {
        let has_content = file.content_hash.is_some()
            || file.content_fingerprint.is_some()
            || file.binary_checked;
        Self {
            relative: file.relative.clone().into_boxed_str(),
            bytes: file.bytes,
            content: has_content.then(|| {
                Box::new(CompactContentEvidence {
                    content_hash: file.content_hash.clone().map(String::into_boxed_str),
                    content_fingerprint: file
                        .content_fingerprint
                        .clone()
                        .map(String::into_boxed_str),
                    version: file.version,
                    binary_checked: file.binary_checked,
                })
            }),
        }
    }
}

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
            descriptor: self.descriptor,
            complete: self.complete,
            termination: self.termination,
            portable: self.portable,
            cache: self.cache,
            record_skipped: true,
        }
    }
}
