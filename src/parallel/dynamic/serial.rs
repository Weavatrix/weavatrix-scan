use super::{
    BatchControl, CancellationToken, ErrorPolicy, ParallelVisitReport, Path, WalkControl,
    WalkEntry, WalkError, WalkOptions, Walker,
};

pub(super) fn visit_batched_serial<F>(
    root: &Path,
    mut options: WalkOptions,
    cancellation: &CancellationToken,
    visitor: F,
) -> Result<ParallelVisitReport, WalkError>
where
    F: Fn(&[WalkEntry], &[WalkError]) -> BatchControl,
{
    let error_policy = options.error_policy;
    options.error_policy = ErrorPolicy::Continue;
    let mut walker = Walker::with_options(root, options)?;
    let mut visited = 0_u64;
    let mut errors = Vec::new();
    let mut quit = false;
    while !cancellation.is_cancelled() && !quit {
        let Some(item) = walker.next() else {
            break;
        };
        match item {
            Ok(entry) => {
                visited = visited.saturating_add(1);
                let decision = visitor(std::slice::from_ref(&entry), &[]);
                let control = decision
                    .entries
                    .first()
                    .copied()
                    .unwrap_or(WalkControl::Continue);
                if control == WalkControl::Skip && entry.is_dir() {
                    walker.skip_current_dir();
                }
                quit = decision.quit || control == WalkControl::Quit;
            }
            Err(error) => {
                let decision = visitor(&[], std::slice::from_ref(&error));
                errors.push(error);
                if error_policy == ErrorPolicy::Abort {
                    return Err(errors.remove(0));
                }
                quit = decision.quit;
            }
        }
    }
    Ok(ParallelVisitReport {
        visited,
        errors,
        quit,
        cancelled: cancellation.is_cancelled(),
    })
}

pub(super) fn stream_batched_serial<F>(
    root: &Path,
    mut options: WalkOptions,
    cancellation: &CancellationToken,
    visitor: F,
) -> Result<ParallelVisitReport, WalkError>
where
    F: Fn(Vec<WalkEntry>, &[WalkError]) -> bool,
{
    let error_policy = options.error_policy;
    options.error_policy = ErrorPolicy::Continue;
    let mut walker = Walker::with_options(root, options)?;
    let mut visited = 0_u64;
    let mut errors = Vec::new();
    let mut quit = false;
    while !cancellation.is_cancelled() && !quit {
        let Some(item) = walker.next() else {
            break;
        };
        match item {
            Ok(entry) => {
                visited = visited.saturating_add(1);
                quit = !visitor(vec![entry], &[]);
            }
            Err(error) => {
                quit = !visitor(Vec::new(), std::slice::from_ref(&error));
                errors.push(error);
                if error_policy == ErrorPolicy::Abort {
                    return Err(errors.remove(0));
                }
            }
        }
    }
    Ok(ParallelVisitReport {
        visited,
        errors,
        quit,
        cancelled: cancellation.is_cancelled(),
    })
}
