use crate::config::{EvidenceMode, IgnorePolicy, ScanOptions, StandardSkips};
use crate::hash::FingerprintHasher;
use crate::report::{IgnoreSourceKind, ScanTermination};
use crate::walk_types::{ErrorPolicy, RootSymlinkPolicy};

/// Version of the semantic scan descriptor stored on reports.
pub const SCAN_DESCRIPTOR_VERSION: u32 = 1;

/// Versioned selection and content-policy identity.
///
/// This is not a completeness flag and is not a substitute for [`crate::ScanReport::revision`].
/// Cancellation tokens and worker scheduling are excluded on purpose.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScanDescriptor {
    /// Descriptor schema version. `0` means a deserialized report omitted it.
    pub version: u32,
    /// Canonical fingerprint of selection, ignore, and content policy.
    pub policy: String,
}

impl ScanDescriptor {
    #[must_use]
    pub fn from_options(options: &ScanOptions) -> Self {
        Self {
            version: SCAN_DESCRIPTOR_VERSION,
            policy: policy_fingerprint(options),
        }
    }

    #[must_use]
    pub fn matches(&self, options: &ScanOptions) -> bool {
        self.version == SCAN_DESCRIPTOR_VERSION && self.policy == policy_fingerprint(options)
    }
}

fn policy_fingerprint(options: &ScanOptions) -> String {
    let mut hasher = FingerprintHasher::new();
    hasher.write(b"scan-policy");
    hasher.write(&SCAN_DESCRIPTOR_VERSION.to_le_bytes());
    write_strings(&mut hasher, options.extensions.iter().map(String::as_str));
    options.file_types.write_policy(&mut hasher);
    write_strings(
        &mut hasher,
        options.override_rules.iter().map(String::as_str),
    );
    write_strings(&mut hasher, options.ignore_files.iter().map(String::as_str));
    write_ignore_policy(&mut hasher, &options.ignore_policy);
    hasher.write(&[u8::from(options.ignore_case_insensitive)]);
    hasher.write(&[u8::from(options.skip_hidden)]);
    hasher.write(&[match options.standard_skips {
        StandardSkips::Enabled => 1,
        StandardSkips::Disabled => 0,
    }]);
    hasher.write(&[u8::from(options.hash_file_contents)]);
    hasher.write(&[u8::from(options.detect_binary_files)]);
    hasher.write(&[match options.evidence {
        EvidenceMode::Complete => 1,
        EvidenceMode::SelectedFiles => 2,
    }]);
    hasher.write(&options.max_file_bytes.to_le_bytes());
    write_optional_u64(&mut hasher, options.limits.max_entries);
    write_optional_u64(&mut hasher, options.limits.max_total_bytes);
    hasher.write(&options.walk.min_depth.to_le_bytes());
    write_optional_usize(&mut hasher, options.walk.max_depth);
    hasher.write(&[u8::from(options.walk.follow_links)]);
    hasher.write(&[u8::from(options.walk.same_file_system)]);
    hasher.write(&[match options.walk.root_symlink_policy {
        RootSymlinkPolicy::Follow => 1,
        RootSymlinkPolicy::Reject => 2,
    }]);
    hasher.write(&[match options.walk.error_policy {
        ErrorPolicy::Continue => 1,
        ErrorPolicy::Abort => 2,
    }]);
    hasher.finish()
}

fn write_ignore_policy(hasher: &mut FingerprintHasher, policy: &IgnorePolicy) {
    hasher.write(&[u8::from(policy.parent_rules)]);
    hasher.write(&[u8::from(policy.git_ignore)]);
    hasher.write(&[u8::from(policy.dot_ignore)]);
    hasher.write(&[u8::from(policy.custom_ignore)]);
    hasher.write(&[u8::from(policy.git_exclude)]);
    hasher.write(&[u8::from(policy.git_global)]);
    hasher.write(&[u8::from(policy.require_git)]);
    let mut files = policy
        .explicit_files
        .iter()
        .map(|path| path.to_string_lossy().replace('\\', "/"))
        .collect::<Vec<_>>();
    files.sort();
    write_strings(hasher, files.iter().map(String::as_str));
}

fn write_strings<'a, I>(hasher: &mut FingerprintHasher, values: I)
where
    I: IntoIterator<Item = &'a str>,
{
    for value in values {
        hasher.write(value.as_bytes());
        hasher.write(&[0]);
    }
    hasher.write(&[0xfe]);
}

fn write_optional_u64(hasher: &mut FingerprintHasher, value: Option<u64>) {
    match value {
        Some(value) => {
            hasher.write(&[1]);
            hasher.write(&value.to_le_bytes());
        }
        None => hasher.write(&[0]),
    }
}

fn write_optional_usize(hasher: &mut FingerprintHasher, value: Option<usize>) {
    write_optional_u64(hasher, value.map(|value| value as u64));
}

pub(crate) fn write_ignore_kind(hasher: &mut FingerprintHasher, kind: IgnoreSourceKind) {
    hasher.write(&[match kind {
        IgnoreSourceKind::GitGlobal => 1,
        IgnoreSourceKind::GitExclude => 2,
        IgnoreSourceKind::GitIgnore => 3,
        IgnoreSourceKind::DotIgnore => 4,
        IgnoreSourceKind::Custom => 5,
        IgnoreSourceKind::Explicit => 6,
        IgnoreSourceKind::Override => 7,
    }]);
}

pub(crate) fn write_termination(hasher: &mut FingerprintHasher, termination: ScanTermination) {
    hasher.write(&[match termination {
        ScanTermination::MaxEntries => 1,
        ScanTermination::MaxTotalBytes => 2,
        ScanTermination::Timeout => 3,
        ScanTermination::Cancelled => 4,
    }]);
}

pub(crate) fn write_selected_file(
    hasher: &mut FingerprintHasher,
    relative: &str,
    bytes: u64,
    content_hash: Option<&str>,
) {
    hasher.write(relative.as_bytes());
    hasher.write(&[0]);
    hasher.write(&bytes.to_le_bytes());
    hasher.write(&[0]);
    hasher.write(content_hash.unwrap_or("").as_bytes());
    hasher.write(&[0xff]);
}

#[cfg(test)]
mod tests {
    use super::{SCAN_DESCRIPTOR_VERSION, ScanDescriptor};
    use crate::config::{ScanOptions, StandardSkips};

    #[test]
    fn policy_fingerprint_ignores_cancellation_and_notices_selection() {
        let base = ScanOptions::default().with_extensions(["rs"]);
        let same = ScanOptions::default()
            .with_extensions(["rs"])
            .with_cancellation(crate::CancellationToken::new());
        let changed = ScanOptions::default().with_extensions(["ts"]);
        let skips = ScanOptions::default()
            .with_extensions(["rs"])
            .with_standard_skips(StandardSkips::Disabled);
        assert_eq!(
            ScanDescriptor::from_options(&base),
            ScanDescriptor::from_options(&same)
        );
        assert_ne!(
            ScanDescriptor::from_options(&base),
            ScanDescriptor::from_options(&changed)
        );
        assert_ne!(
            ScanDescriptor::from_options(&base),
            ScanDescriptor::from_options(&skips)
        );
        assert_eq!(
            ScanDescriptor::from_options(&base).version,
            SCAN_DESCRIPTOR_VERSION
        );
    }
}
