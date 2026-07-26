use crate::ignore::RepositoryMatch;
use crate::report::{ScanReport, SkipKind};

pub(crate) fn skip_match(
    report: &mut ScanReport,
    relative: String,
    matched: RepositoryMatch,
) -> bool {
    let Some(kind) = skip_kind_for_match(matched) else {
        return false;
    };
    report.skip(relative, kind, None);
    true
}

pub(crate) const fn skip_kind_for_match(matched: RepositoryMatch) -> Option<SkipKind> {
    match matched {
        RepositoryMatch::Ignore => Some(SkipKind::Ignored),
        RepositoryMatch::OverrideIgnore => Some(SkipKind::Override),
        RepositoryMatch::Hidden => Some(SkipKind::Hidden),
        RepositoryMatch::None | RepositoryMatch::Include | RepositoryMatch::OverrideInclude => None,
    }
}
