use super::entry::{prepare_batch_directory, process_entry_with, record_walk_error};
use crate::config::ScanOptions;
use crate::error::Error;
use crate::ignore::RepositoryMatcher;
use crate::parallel::WalkControl;
use crate::parallel::dynamic::BatchControl;
use crate::report::{FileVersion, ScanReport};
use crate::scan_limits::ScanRuntime;
use crate::walker::{ErrorPolicy, WalkEntry, WalkError};
use std::path::Path;

#[allow(clippy::too_many_arguments)]
pub(super) fn process_parallel_batch<T, F>(
    entries: &[WalkEntry],
    errors: &[WalkError],
    root: &Path,
    options: &ScanOptions,
    report: &mut ScanReport,
    matcher: &mut RepositoryMatcher,
    runtime: &mut ScanRuntime,
    scan_error: &mut Option<Error>,
    selected: &mut Vec<T>,
    mut make_selected: F,
) -> BatchControl
where
    F: FnMut(&Path, String, u64, FileVersion) -> T,
{
    let mut controls = Vec::with_capacity(entries.len());
    let mut quit = scan_error.is_some();
    let prepared_parent = if quit {
        None
    } else {
        match prepare_batch_directory(matcher, entries) {
            Ok(parent) => parent,
            Err(error) => {
                *scan_error = Some(error);
                quit = true;
                None
            }
        }
    };
    let prepared_rules = prepared_parent.map(|parent| matcher.prepared_rules(parent));
    for entry in entries {
        if quit {
            controls.push(WalkControl::Quit);
            continue;
        }
        if let Some(reason) = runtime.before_next(options) {
            report.terminate(reason);
            controls.push(WalkControl::Quit);
            quit = true;
            continue;
        }
        runtime.record_entry();
        let outcome = process_entry_with(
            entry,
            options,
            report,
            matcher,
            prepared_rules,
            |path, relative, bytes, version| {
                selected.push(make_selected(path, relative, bytes, version));
            },
        );
        match outcome {
            Ok(true) => controls.push(WalkControl::Skip),
            Ok(false) => controls.push(WalkControl::Continue),
            Err(error) => {
                *scan_error = Some(error);
                controls.push(WalkControl::Quit);
                quit = true;
            }
        }
    }
    for error in errors {
        if quit {
            break;
        }
        if let Some(reason) = runtime.before_next(options) {
            report.terminate(reason);
            quit = true;
            break;
        }
        runtime.record_entry();
        if options.walk.error_policy == ErrorPolicy::Continue {
            record_walk_error(error, root, report);
        }
    }
    BatchControl {
        entries: controls,
        quit,
    }
}
