use crate::config::ScanOptions;
use crate::error::{Error, Result};
use crate::path::normalized_relative_path;
use std::collections::{BTreeSet, HashSet};
use std::path::{Component, Path, PathBuf};

/// Normalized watcher event category independent of a watcher implementation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WatchEventKind {
    Create,
    Modify,
    Remove,
    RenameFrom,
    RenameTo,
    Directory,
    Rescan,
}

/// One raw path event supplied by a filesystem watcher.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WatchEvent {
    pub path: PathBuf,
    pub kind: WatchEventKind,
}

impl WatchEvent {
    #[must_use]
    pub fn new(path: impl Into<PathBuf>, kind: WatchEventKind) -> Self {
        Self {
            path: path.into(),
            kind,
        }
    }
}

/// Deterministic invalidation plan derived from watcher events.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WatchPlan {
    pub changed: Vec<String>,
    pub removed: Vec<String>,
    /// Directory, ignore-input, root, or explicit rescan event was observed.
    pub full_rescan: bool,
    /// Events outside the configured root or containing traversal components.
    pub rejected_events: u64,
}

impl WatchPlan {
    pub fn invalidated(&self) -> impl Iterator<Item = &str> {
        self.changed.iter().chain(&self.removed).map(String::as_str)
    }
}

/// Converts watcher-specific path notifications into scanner cache work.
pub struct WatcherEventAdapter {
    root: PathBuf,
    event_root: PathBuf,
    ignore_files: BTreeSet<String>,
}

impl WatcherEventAdapter {
    /// Creates an adapter for an existing repository root.
    ///
    /// # Errors
    ///
    /// Returns an error when the root cannot be canonicalized or is not a
    /// directory.
    pub fn new<I, S>(root: impl AsRef<Path>, ignore_files: I) -> Result<Self>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let requested = root.as_ref();
        let event_root = if requested.is_absolute() {
            requested.to_path_buf()
        } else {
            std::env::current_dir()
                .map_err(|source| Error::io(requested, source))?
                .join(requested)
        };
        let root = requested
            .canonicalize()
            .map_err(|source| Error::io(requested, source))?;
        if !root.is_dir() {
            return Err(Error::InvalidRoot(root));
        }
        Ok(Self {
            root,
            event_root,
            ignore_files: ignore_files
                .into_iter()
                .map(|name| name.as_ref().replace('\\', "/"))
                .collect(),
        })
    }

    /// Creates an adapter using the scanner's configured ignore filenames.
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`Self::new`].
    pub fn with_options(root: impl AsRef<Path>, options: &ScanOptions) -> Result<Self> {
        Self::new(root, &options.ignore_files)
    }

    /// Coalesces raw watcher events into a stable scanner invalidation plan.
    #[must_use]
    pub fn plan<I>(&self, events: I) -> WatchPlan
    where
        I: IntoIterator<Item = WatchEvent>,
    {
        let mut changed = HashSet::new();
        let mut removed = HashSet::new();
        let mut full_rescan = false;
        let mut rejected_events = 0_u64;

        for event in events {
            if event.kind == WatchEventKind::Rescan {
                full_rescan = true;
                continue;
            }
            let Some(relative) = self.relative(&event.path) else {
                rejected_events = rejected_events.saturating_add(1);
                continue;
            };
            if relative.is_empty()
                || event.kind == WatchEventKind::Directory
                || self.controls_selection(&relative)
            {
                full_rescan = true;
                continue;
            }
            match event.kind {
                WatchEventKind::Create | WatchEventKind::Modify | WatchEventKind::RenameTo => {
                    removed.remove(&relative);
                    changed.insert(relative);
                }
                WatchEventKind::Remove | WatchEventKind::RenameFrom => {
                    changed.remove(&relative);
                    removed.insert(relative);
                }
                WatchEventKind::Directory | WatchEventKind::Rescan => {
                    full_rescan = true;
                }
            }
        }

        let mut changed = changed.into_iter().collect::<Vec<_>>();
        let mut removed = removed.into_iter().collect::<Vec<_>>();
        changed.sort_unstable();
        removed.sort_unstable();
        WatchPlan {
            changed,
            removed,
            full_rescan,
            rejected_events,
        }
    }

    fn relative(&self, path: &Path) -> Option<String> {
        let relative = if path.is_absolute() {
            strip_root(path, &self.event_root)
                .or_else(|| strip_root(path, &self.root))
                .or_else(|| strip_canonical_root(path, &self.root))?
        } else {
            path.to_path_buf()
        };
        if relative.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        }) {
            return None;
        }
        Some(normalized_relative_path(&relative))
    }

    fn controls_selection(&self, relative: &str) -> bool {
        if matches!(relative, ".git/config" | ".git/info/exclude") {
            return true;
        }
        let file_name = Path::new(relative)
            .file_name()
            .and_then(|name| name.to_str());
        file_name.is_some_and(|file_name| {
            self.ignore_files.iter().any(|configured| {
                Path::new(configured)
                    .file_name()
                    .and_then(|name| name.to_str())
                    == Some(file_name)
            })
        })
    }
}

fn strip_root(path: &Path, root: &Path) -> Option<PathBuf> {
    if let Ok(relative) = path.strip_prefix(root) {
        return Some(relative.to_path_buf());
    }
    #[cfg(windows)]
    if let Some(relative) = strip_windows_root(path, root) {
        return Some(relative);
    }
    None
}

fn strip_canonical_root(path: &Path, root: &Path) -> Option<PathBuf> {
    if let Ok(canonical) = path.canonicalize()
        && let Ok(relative) = canonical.strip_prefix(root)
    {
        return Some(relative.to_path_buf());
    }
    None
}

#[cfg(windows)]
fn strip_windows_root(path: &Path, root: &Path) -> Option<PathBuf> {
    let mut path_components = path.components();
    for root_component in root.components() {
        let path_component = path_components.next()?;
        if !windows_component_eq(path_component, root_component) {
            return None;
        }
    }
    Some(path_components.collect())
}

#[cfg(windows)]
fn windows_component_eq(left: Component<'_>, right: Component<'_>) -> bool {
    let normalize = |component: Component<'_>| {
        let value = component.as_os_str().to_string_lossy();
        value.strip_prefix(r"\\?\UNC\").map_or_else(
            || value.strip_prefix(r"\\?\").unwrap_or(&value).to_owned(),
            |suffix| format!(r"\\{suffix}"),
        )
    };
    normalize(left).eq_ignore_ascii_case(&normalize(right))
}
