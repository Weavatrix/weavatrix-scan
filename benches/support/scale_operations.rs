use ignore::WalkBuilder;
use jwalk::WalkDir as JWalkDir;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use walkdir::WalkDir;
use weavatrix_scan::{
    ContentFileStatus, ContentVisitControl, ContentVisitEvent, ParallelWalker, ScanOptions,
    Scanner, StandardSkips, WalkControl, WalkEntry, WalkEvent, Walker,
};

pub(super) fn walker(root: &Path) -> usize {
    Walker::new(root)
        .unwrap()
        .filter_map(Result::ok)
        .filter(WalkEntry::is_file)
        .filter(|entry| is_source(entry.path()))
        .count()
}

pub(super) fn parallel_collected(root: &Path) -> usize {
    ParallelWalker::new(root)
        .walk()
        .unwrap()
        .entries
        .into_iter()
        .filter(WalkEntry::is_file)
        .filter(|entry| is_source(entry.path()))
        .count()
}

pub(super) fn parallel_stream(root: &Path) -> usize {
    let count = Arc::new(AtomicUsize::new(0));
    let visitor_count = Arc::clone(&count);
    ParallelWalker::new(root)
        .visit(move |event| {
            if let WalkEvent::Entry(entry) = event
                && entry.is_file()
                && is_source(entry.path())
            {
                visitor_count.fetch_add(1, Ordering::Relaxed);
            }
            WalkControl::Continue
        })
        .unwrap();
    count.load(Ordering::Relaxed)
}

pub(super) fn jwalk(root: &Path) -> usize {
    JWalkDir::new(root)
        .sort(false)
        .skip_hidden(false)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .filter(|entry| is_source(&entry.path()))
        .count()
}

pub(super) fn walkdir(root: &Path) -> usize {
    WalkDir::new(root)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .filter(|entry| is_source(entry.path()))
        .count()
}

pub(super) fn ignore(root: &Path) -> usize {
    ignore_builder(root)
        .build()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_some_and(|kind| kind.is_file()))
        .filter(|entry| is_source(entry.path()))
        .count()
}

pub(super) fn ignore_manifest(root: &Path) -> usize {
    ignore_manifest_entries(root).len()
}

pub(super) fn ignore_manifest_entries(root: &Path) -> Vec<(String, u64)> {
    let mut files = ignore_builder(root)
        .build()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_some_and(|kind| kind.is_file()))
        .filter(|entry| is_source(entry.path()))
        .map(|entry| {
            let bytes = entry.metadata().unwrap().len();
            let relative = entry
                .path()
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            (relative, bytes)
        })
        .collect::<Vec<_>>();
    files.sort_unstable();
    files
}

fn ignore_builder(root: &Path) -> WalkBuilder {
    let mut builder = WalkBuilder::new(root);
    builder
        .hidden(false)
        .git_global(false)
        .git_exclude(false)
        .parents(false)
        .require_git(false);
    builder
}

pub(super) fn scanner(root: &Path) -> usize {
    scanner_with_options(
        root,
        ScanOptions::default()
            .with_extensions(["rs"])
            .metadata_only()
            .selected_files_only(),
    )
}

pub(super) fn scanner_compact(root: &Path) -> usize {
    Scanner::new(root)
        .options(
            ScanOptions::default()
                .with_extensions(["rs"])
                .metadata_only()
                .selected_files_only(),
        )
        .scan_compact()
        .unwrap()
        .files
        .len()
}

pub(super) fn scanner_all(root: &Path) -> usize {
    let mut options = ScanOptions::default()
        .with_extensions(["rs"])
        .with_ignore_files(std::iter::empty::<&str>())
        .metadata_only()
        .selected_files_only();
    options.standard_skips = StandardSkips::Disabled;
    scanner_with_options(root, options)
}

pub(super) fn scanner_rich(root: &Path) -> usize {
    scanner_with_options(
        root,
        ScanOptions::default()
            .with_extensions(["rs"])
            .selected_files_only(),
    )
}

pub(super) fn content_visit(root: &Path, streaming: bool) -> usize {
    let selected = Arc::new(AtomicUsize::new(0));
    let scanner = Scanner::new(root).options(
        ScanOptions::default()
            .with_extensions(["rs"])
            .selected_files_only(),
    );
    let report = if streaming {
        scanner.visit_content_streaming({
            let selected = Arc::clone(&selected);
            move |_| {
                let selected = Arc::clone(&selected);
                move |event| count_selected_content(&event, &selected)
            }
        })
    } else {
        scanner.visit_content({
            let selected = Arc::clone(&selected);
            move |_| {
                let selected = Arc::clone(&selected);
                move |event| count_selected_content(&event, &selected)
            }
        })
    }
    .unwrap();
    let selected = selected.load(Ordering::Relaxed);
    assert_eq!(usize::try_from(report.completed).unwrap(), selected);
    selected
}

fn count_selected_content(
    event: &ContentVisitEvent<'_>,
    selected: &AtomicUsize,
) -> ContentVisitControl {
    if let ContentVisitEvent::FileEnd {
        status: ContentFileStatus::Selected,
        ..
    } = event
    {
        selected.fetch_add(1, Ordering::Relaxed);
    }
    ContentVisitControl::Continue
}

fn scanner_with_options(root: &Path, options: ScanOptions) -> usize {
    scanner_manifest(root, options).len()
}

pub(super) fn scanner_manifest(root: &Path, options: ScanOptions) -> Vec<(String, u64)> {
    Scanner::new(root)
        .options(options)
        .scan()
        .unwrap()
        .files
        .into_iter()
        .map(|file| (file.relative, file.bytes))
        .collect()
}

fn is_source(path: &Path) -> bool {
    path.extension().is_some_and(|extension| extension == "rs")
}
