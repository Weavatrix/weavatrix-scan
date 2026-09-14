use super::super::entry::{prepare_batch_directory, walker_error_into_scan_error};
use super::decide::{PathAction, decide_path};
use super::{finish, stop_error};
use crate::config::ScanOptions;
use crate::error::{Error, Result};
use crate::ignore::RepositoryMatcher;
use crate::parallel::WalkControl;
use crate::parallel::dynamic::{self, BatchControl};
use crate::runtime::ParallelRuntime;
use crate::scan_limits::ScanRuntime;
use crate::walk_types::{ErrorPolicy, WalkEntry, WalkError, WalkOptions};
use crate::walker::Walker;
use std::path::Path;
use std::sync::{Arc, Mutex};

pub(super) fn paths_walk_options(options: &ScanOptions) -> WalkOptions {
    let mut walk = options.walk_options();
    walk.collect_metadata = false;
    walk
}

pub(super) fn discover_serial(
    canonical: &Path,
    options: &ScanOptions,
    mut matcher: RepositoryMatcher,
    mut runtime: ScanRuntime,
) -> Result<(Vec<String>, ScanRuntime)> {
    let mut paths = Vec::new();
    let mut walker = Walker::with_options(canonical, paths_walk_options(options))
        .map_err(walker_error_into_scan_error)?;
    loop {
        if let Some(reason) = runtime.before_next(options) {
            return finish(canonical, paths, runtime, reason);
        }
        let Some(item) = walker.next() else {
            break;
        };
        runtime.record_entry();
        match item {
            Ok(entry) => match decide_path(&entry, options, &matcher, None) {
                PathAction::SkipDir => walker.skip_current_dir(),
                PathAction::Continue => {
                    if entry.is_dir() {
                        matcher.prepare_directory(entry.path())?;
                    }
                }
                PathAction::File(relative) => paths.push(relative),
            },
            Err(error) if options.walk.error_policy == ErrorPolicy::Abort => {
                return Err(walker_error_into_scan_error(error));
            }
            Err(_) => {}
        }
    }
    Ok((paths, runtime))
}

struct ParallelPaths {
    paths: Vec<String>,
    matcher: RepositoryMatcher,
    runtime: ScanRuntime,
    error: Option<Error>,
}

pub(super) fn discover_parallel(
    canonical: &Path,
    options: &ScanOptions,
    matcher: RepositoryMatcher,
    runtime: ScanRuntime,
    parallel_runtime: &ParallelRuntime,
) -> Result<(Vec<String>, ScanRuntime)> {
    let state = Arc::new(Mutex::new(ParallelPaths {
        paths: Vec::new(),
        matcher,
        runtime,
        error: None,
    }));
    let visitor_state = Arc::clone(&state);
    let visitor_options = options.clone();
    let cancellation = options.cancellation.clone().unwrap_or_default();
    let traversal = dynamic::visit_batched(
        canonical,
        paths_walk_options(options),
        options.traversal_workers(),
        parallel_runtime,
        &cancellation,
        move |entries, errors| {
            let mut state = visitor_state
                .lock()
                .expect("path scanner state is not poisoned");
            collect_parallel_batch(entries, errors, &visitor_options, &mut state)
        },
    );
    if let Err(error) = traversal {
        return Err(walker_error_into_scan_error(error));
    }
    let state = Arc::try_unwrap(state)
        .ok()
        .expect("path scanner visitor released shared state");
    let mut state = state
        .into_inner()
        .expect("path scanner state is not poisoned");
    if let Some(error) = state.error.take() {
        return Err(error);
    }
    Ok((state.paths, state.runtime))
}

fn collect_parallel_batch(
    entries: &[WalkEntry],
    errors: &[WalkError],
    options: &ScanOptions,
    state: &mut ParallelPaths,
) -> BatchControl {
    if state.error.is_some() {
        return quit_all(entries.len());
    }
    let prepared_parent = match prepare_batch_directory(&mut state.matcher, entries) {
        Ok(parent) => parent,
        Err(error) => {
            state.error = Some(error);
            return quit_all(entries.len());
        }
    };
    let prepared_rules = prepared_parent.map(|parent| state.matcher.prepared_rules(parent));
    let mut controls = Vec::with_capacity(entries.len());
    let mut quit = false;
    for entry in entries {
        if quit {
            controls.push(WalkControl::Quit);
            continue;
        }
        if let Some(reason) = state.runtime.before_next(options) {
            if let Some(error) = stop_error(entry.path(), reason) {
                state.error = Some(error);
            }
            controls.push(WalkControl::Quit);
            quit = true;
            continue;
        }
        state.runtime.record_entry();
        match decide_path(entry, options, &state.matcher, prepared_rules) {
            PathAction::SkipDir => controls.push(WalkControl::Skip),
            PathAction::Continue => controls.push(WalkControl::Continue),
            PathAction::File(relative) => {
                state.paths.push(relative);
                controls.push(WalkControl::Continue);
            }
        }
    }
    if options.walk.error_policy == ErrorPolicy::Abort
        && let Some(error) = errors.first()
    {
        state.error = Some(Error::io(
            error.path(),
            std::io::Error::new(error.io_error().kind(), error.io_error().to_string()),
        ));
        quit = true;
    }
    BatchControl {
        entries: controls,
        quit,
    }
}

fn quit_all(count: usize) -> BatchControl {
    BatchControl {
        entries: vec![WalkControl::Quit; count],
        quit: true,
    }
}
