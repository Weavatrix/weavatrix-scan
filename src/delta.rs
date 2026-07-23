use crate::report::{ScanReport, ScannedFile};
use std::collections::{BTreeMap, BTreeSet};

/// Strength of the evidence used to classify a scan delta.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeltaQuality {
    /// Both complete reports contain a content hash for every selected file.
    ContentHash,
    /// At least one file was compared by size because a content hash was absent.
    Metadata,
    /// At least one report is partial or terminated.
    Partial,
}

/// A selected file whose stable relative path remained but content changed.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModifiedFile {
    pub previous: ScannedFile,
    pub current: ScannedFile,
}

/// A uniquely content-matched file whose stable relative path changed.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenamedFile {
    pub previous: ScannedFile,
    pub current: ScannedFile,
}

/// Deterministic changes between two repository manifests.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanDelta {
    pub from_revision: String,
    pub to_revision: String,
    pub added: Vec<ScannedFile>,
    pub removed: Vec<ScannedFile>,
    pub modified: Vec<ModifiedFile>,
    pub renamed: Vec<RenamedFile>,
    pub unchanged: u64,
    pub selection_inputs_changed: bool,
    pub scan_state_changed: bool,
    pub quality: DeltaQuality,
}

impl ScanDelta {
    #[must_use]
    pub fn between(previous: &ScanReport, current: &ScanReport) -> Self {
        let quality = delta_quality(previous, current);
        let mut previous_files = previous.files.iter().collect::<Vec<_>>();
        let mut current_files = current.files.iter().collect::<Vec<_>>();
        previous_files.sort_unstable_by(|left, right| left.relative.cmp(&right.relative));
        current_files.sort_unstable_by(|left, right| left.relative.cmp(&right.relative));
        let mut delta = Self {
            from_revision: previous.revision.clone(),
            to_revision: current.revision.clone(),
            added: Vec::new(),
            removed: Vec::new(),
            modified: Vec::new(),
            renamed: Vec::new(),
            unchanged: 0,
            selection_inputs_changed: previous.ignore_sources != current.ignore_sources,
            scan_state_changed: previous.root != current.root
                || previous.complete != current.complete
                || previous.termination != current.termination
                || previous.portable != current.portable,
            quality,
        };
        merge_files(&previous_files, &current_files, &mut delta);
        detect_unique_renames(previous, current, &mut delta);
        delta
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.added.is_empty()
            && self.removed.is_empty()
            && self.modified.is_empty()
            && self.renamed.is_empty()
            && !self.selection_inputs_changed
            && !self.scan_state_changed
    }
}

fn delta_quality(previous: &ScanReport, current: &ScanReport) -> DeltaQuality {
    if !previous.complete
        || !current.complete
        || previous.termination.is_some()
        || current.termination.is_some()
    {
        return DeltaQuality::Partial;
    }
    if previous
        .files
        .iter()
        .chain(&current.files)
        .any(|file| file.content_hash.is_none())
    {
        DeltaQuality::Metadata
    } else {
        DeltaQuality::ContentHash
    }
}

fn merge_files(previous: &[&ScannedFile], current: &[&ScannedFile], delta: &mut ScanDelta) {
    let (mut previous_index, mut current_index) = (0, 0);
    while previous_index < previous.len() || current_index < current.len() {
        match (previous.get(previous_index), current.get(current_index)) {
            (Some(before), Some(after)) if before.relative == after.relative => {
                if same_content(before, after) {
                    delta.unchanged = delta.unchanged.saturating_add(1);
                } else {
                    delta.modified.push(ModifiedFile {
                        previous: (*before).clone(),
                        current: (*after).clone(),
                    });
                }
                previous_index += 1;
                current_index += 1;
            }
            (Some(before), Some(after)) if before.relative < after.relative => {
                delta.removed.push((*before).clone());
                previous_index += 1;
            }
            (Some(_) | None, Some(after)) => {
                delta.added.push((*after).clone());
                current_index += 1;
            }
            (Some(before), None) => {
                delta.removed.push((*before).clone());
                previous_index += 1;
            }
            (None, None) => break,
        }
    }
}

fn same_content(previous: &ScannedFile, current: &ScannedFile) -> bool {
    previous.bytes == current.bytes
        && match (&previous.content_hash, &current.content_hash) {
            (Some(previous), Some(current)) => previous == current,
            _ => true,
        }
}

fn detect_unique_renames(previous: &ScanReport, current: &ScanReport, delta: &mut ScanDelta) {
    let previous_counts = hash_counts(&previous.files);
    let current_counts = hash_counts(&current.files);
    let added_by_hash = unique_indices(&delta.added);
    let removed_by_hash = unique_indices(&delta.removed);
    let mut added_renames = vec![false; delta.added.len()];
    let mut removed_renames = vec![false; delta.removed.len()];
    for (hash, &removed_index) in &removed_by_hash {
        let Some(&added_index) = added_by_hash.get(hash) else {
            continue;
        };
        if previous_counts.get(hash) != Some(&1) || current_counts.get(hash) != Some(&1) {
            continue;
        }
        removed_renames[removed_index] = true;
        added_renames[added_index] = true;
        delta.renamed.push(RenamedFile {
            previous: delta.removed[removed_index].clone(),
            current: delta.added[added_index].clone(),
        });
    }
    delta.renamed.sort_unstable_by(|left, right| {
        left.previous
            .relative
            .cmp(&right.previous.relative)
            .then_with(|| left.current.relative.cmp(&right.current.relative))
    });
    retain_unmarked(&mut delta.added, &added_renames);
    retain_unmarked(&mut delta.removed, &removed_renames);
}

fn hash_counts(files: &[ScannedFile]) -> BTreeMap<&str, usize> {
    let mut counts = BTreeMap::new();
    for hash in files.iter().filter_map(|file| file.content_hash.as_deref()) {
        *counts.entry(hash).or_default() += 1;
    }
    counts
}

fn unique_indices(files: &[ScannedFile]) -> BTreeMap<&str, usize> {
    let mut indices = BTreeMap::new();
    let mut duplicates = BTreeSet::new();
    for (index, hash) in files
        .iter()
        .enumerate()
        .filter_map(|(index, file)| file.content_hash.as_deref().map(|hash| (index, hash)))
    {
        if indices.insert(hash, index).is_some() {
            duplicates.insert(hash);
        }
    }
    indices.retain(|hash, _| !duplicates.contains(hash));
    indices
}

fn retain_unmarked(files: &mut Vec<ScannedFile>, marked: &[bool]) {
    let mut index = 0;
    files.retain(|_| {
        let retain = !marked[index];
        index += 1;
        retain
    });
}
