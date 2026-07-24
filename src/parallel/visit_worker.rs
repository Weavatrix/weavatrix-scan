use super::visit::{ParallelVisitReport, WalkControl, WalkEvent};
use crate::control::CancellationToken;
use crate::walker::{ErrorPolicy, WalkError, WalkOptions, Walker};
use std::path::Path;

pub(super) fn visit_serial<F>(
    root: &Path,
    options: WalkOptions,
    cancellation: &CancellationToken,
    visitor: F,
) -> Result<ParallelVisitReport, WalkError>
where
    F: for<'entry> Fn(WalkEvent<'entry>) -> WalkControl + Sync,
{
    let mut walker_options = options;
    walker_options.error_policy = ErrorPolicy::Continue;
    let mut walker = Walker::with_options(root, walker_options)?;
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
                match visitor(WalkEvent::Entry(&entry)) {
                    WalkControl::Skip if entry.is_dir() => walker.skip_current_dir(),
                    WalkControl::Continue | WalkControl::Skip => {}
                    WalkControl::Quit => quit = true,
                }
            }
            Err(error) => {
                let control = visitor(WalkEvent::Error(&error));
                errors.push(error);
                if options.error_policy == ErrorPolicy::Abort {
                    return Err(errors.remove(0));
                }
                quit = control == WalkControl::Quit;
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
