use crate::ignore::RepositoryMatch;
use crate::report::{ScanReport, SkipKind};

pub(crate) fn skip_match(
    report: &mut ScanReport,
    relative: String,
    matched: RepositoryMatch,
) -> bool {
    let kind = match matched {
        RepositoryMatch::Ignore => SkipKind::Ignored,
        RepositoryMatch::OverrideIgnore => SkipKind::Override,
        RepositoryMatch::Hidden => SkipKind::Hidden,
        RepositoryMatch::None | RepositoryMatch::Include | RepositoryMatch::OverrideInclude => {
            return false;
        }
    };
    report.skip(relative, kind, None);
    true
}
