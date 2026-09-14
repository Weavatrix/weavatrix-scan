use crate::path::is_same_or_descendant;
use crate::report::{IgnoreSourceEvidence, IgnoreSourceKind};
use crate::watch::WatchPlan;

pub(super) fn ignore_sources_changed(
    current: &[IgnoreSourceEvidence],
    previous: &[IgnoreSourceEvidence],
    plan: &WatchPlan,
) -> bool {
    current
        .iter()
        .any(|source| !previous.iter().any(|known| known == source))
        || previous
            .iter()
            .any(|known| missing_loaded_source(known, current, plan))
}

fn missing_loaded_source(
    known: &IgnoreSourceEvidence,
    current: &[IgnoreSourceEvidence],
    plan: &WatchPlan,
) -> bool {
    if current
        .iter()
        .any(|source| source.kind == known.kind && source.location == known.location)
    {
        return false;
    }
    would_have_loaded(known, plan)
}

fn would_have_loaded(source: &IgnoreSourceEvidence, plan: &WatchPlan) -> bool {
    if always_loaded(source) {
        return true;
    }
    let Some((directory, _)) = source.location.rsplit_once('/') else {
        return true;
    };
    plan.invalidated().any(|relative| {
        is_same_or_descendant(relative, directory) || is_same_or_descendant(directory, relative)
    })
}

fn always_loaded(source: &IgnoreSourceEvidence) -> bool {
    matches!(
        source.kind,
        IgnoreSourceKind::GitGlobal
            | IgnoreSourceKind::GitExclude
            | IgnoreSourceKind::Explicit
            | IgnoreSourceKind::Override
    ) || source.location.starts_with('<')
        || looks_absolute(&source.location)
}

fn looks_absolute(location: &str) -> bool {
    let bytes = location.as_bytes();
    location.starts_with(['/', '\\'])
        || bytes.get(1) == Some(&b':') && bytes.first().is_some_and(u8::is_ascii_alphabetic)
}
