use crate::config::ScanOptions;
use crate::ignore::{IgnoreRules, RepositoryMatch, RepositoryMatcher};
use crate::path::normalized_relative_path;
use crate::scan_match::skip_kind_for_match;
use crate::walk_types::WalkEntry;

pub(super) enum PathAction {
    Continue,
    SkipDir,
    File(String),
}

pub(super) fn decide_path(
    entry: &WalkEntry,
    options: &ScanOptions,
    matcher: &RepositoryMatcher,
    prepared_rules: Option<&IgnoreRules>,
) -> PathAction {
    if entry.depth() == 0 {
        return PathAction::Continue;
    }
    if entry.skip_reason().is_some() {
        return if entry.is_dir() {
            PathAction::SkipDir
        } else {
            PathAction::Continue
        };
    }
    if entry.is_symlink() && !options.walk.follow_links {
        return PathAction::Continue;
    }
    if entry.depth() < options.effective_min_depth() && !entry.is_dir() {
        return PathAction::Continue;
    }
    let relative = normalized_relative_path(entry.relative_path());
    if entry.is_dir() {
        return decide_directory(entry, &relative, options, matcher, prepared_rules);
    }
    if entry.is_file() {
        decide_file(entry, relative, options, matcher, prepared_rules)
    } else {
        PathAction::Continue
    }
}

fn decide_directory(
    entry: &WalkEntry,
    relative: &str,
    options: &ScanOptions,
    matcher: &RepositoryMatcher,
    prepared_rules: Option<&IgnoreRules>,
) -> PathAction {
    let decision = match_entry(entry, relative, true, matcher, prepared_rules);
    if skip_kind_for_match(decision).is_some() {
        return PathAction::SkipDir;
    }
    if decision != RepositoryMatch::OverrideInclude
        && options.should_skip_directory(entry.file_name())
    {
        return PathAction::SkipDir;
    }
    PathAction::Continue
}

fn decide_file(
    entry: &WalkEntry,
    relative: String,
    options: &ScanOptions,
    matcher: &RepositoryMatcher,
    prepared_rules: Option<&IgnoreRules>,
) -> PathAction {
    let decision = match_entry(entry, &relative, false, matcher, prepared_rules);
    if skip_kind_for_match(decision).is_some() {
        return PathAction::Continue;
    }
    if decision != RepositoryMatch::OverrideInclude
        && !options.accepts_extension(entry.path(), &relative)
    {
        return PathAction::Continue;
    }
    PathAction::File(relative)
}

fn match_entry(
    entry: &WalkEntry,
    relative: &str,
    is_directory: bool,
    matcher: &RepositoryMatcher,
    prepared_rules: Option<&IgnoreRules>,
) -> RepositoryMatch {
    let parent = entry.path().parent().unwrap_or(entry.path());
    prepared_rules.map_or_else(
        || {
            matcher.matched_prepared(
                relative,
                parent,
                entry.path(),
                is_directory,
                entry.hidden(),
                true,
            )
        },
        |rules| {
            matcher.matched_with_rules(
                relative,
                entry.path(),
                is_directory,
                entry.hidden(),
                rules,
                true,
            )
        },
    )
}
