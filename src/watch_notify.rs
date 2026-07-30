use crate::watch::{WatchEvent, WatchEventKind, WatchPlan, WatcherEventAdapter};
use notify::event::{CreateKind, ModifyKind, RemoveKind, RenameMode};
use notify::{Event, EventKind};
use std::path::PathBuf;

impl WatcherEventAdapter {
    /// Converts native `notify` events into a deterministic scanner plan.
    ///
    /// Access-only events are ignored. Imprecise, rescan, and possibly
    /// structural events conservatively request a complete scan.
    #[must_use]
    pub fn plan_notify<I>(&self, events: I) -> WatchPlan
    where
        I: IntoIterator<Item = Event>,
    {
        self.plan(events.into_iter().flat_map(convert_event))
    }
}

fn convert_event(event: Event) -> Vec<WatchEvent> {
    if event.need_rescan() {
        return rescan();
    }
    match event.kind {
        EventKind::Access(_) => Vec::new(),
        EventKind::Any
        | EventKind::Other
        | EventKind::Remove(RemoveKind::Any | RemoveKind::Folder | RemoveKind::Other)
        | EventKind::Modify(ModifyKind::Any | ModifyKind::Other) => rescan(),
        EventKind::Create(kind) => map_paths(event.paths, create_kind(kind)),
        EventKind::Remove(RemoveKind::File) => map_paths(event.paths, Some(WatchEventKind::Remove)),
        EventKind::Modify(ModifyKind::Name(mode)) => rename_events(mode, event.paths),
        EventKind::Modify(ModifyKind::Data(_) | ModifyKind::Metadata(_)) => {
            if event.paths.iter().any(|path| path.is_dir()) {
                rescan()
            } else {
                map_paths(event.paths, Some(WatchEventKind::Modify))
            }
        }
    }
}

fn create_kind(kind: CreateKind) -> Option<WatchEventKind> {
    match kind {
        CreateKind::File => Some(WatchEventKind::Create),
        CreateKind::Folder => Some(WatchEventKind::Directory),
        CreateKind::Any | CreateKind::Other => None,
    }
}

fn map_paths(paths: Vec<PathBuf>, kind: Option<WatchEventKind>) -> Vec<WatchEvent> {
    let Some(kind) = kind else {
        return if paths.iter().any(|path| path.is_dir()) {
            paths
                .into_iter()
                .map(|path| WatchEvent::new(path, WatchEventKind::Directory))
                .collect()
        } else if paths.is_empty() {
            rescan()
        } else {
            paths
                .into_iter()
                .map(|path| WatchEvent::new(path, WatchEventKind::Create))
                .collect()
        };
    };
    if paths.is_empty() {
        return rescan();
    }
    paths
        .into_iter()
        .map(|path| WatchEvent::new(path, kind))
        .collect()
}

fn rename_events(mode: RenameMode, paths: Vec<PathBuf>) -> Vec<WatchEvent> {
    match mode {
        RenameMode::From => map_paths(paths, Some(WatchEventKind::RenameFrom)),
        RenameMode::To => map_paths(paths, Some(WatchEventKind::RenameTo)),
        RenameMode::Both | RenameMode::Any | RenameMode::Other => rename_pair_or_rescan(&paths),
    }
}

fn rename_pair_or_rescan(paths: &[PathBuf]) -> Vec<WatchEvent> {
    if paths.len() != 2 {
        return rescan();
    }
    vec![
        WatchEvent::new(paths[0].clone(), WatchEventKind::RenameFrom),
        WatchEvent::new(paths[1].clone(), WatchEventKind::RenameTo),
    ]
}

fn rescan() -> Vec<WatchEvent> {
    vec![WatchEvent::new(PathBuf::new(), WatchEventKind::Rescan)]
}
