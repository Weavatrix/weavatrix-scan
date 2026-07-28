use ignore::WalkBuilder;
use jwalk::WalkDir as JWalkDir;
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;
use weavatrix_scan::{ParallelWalker, WalkEntry, WalkOptions, Walker};

pub type Paths = Vec<PathBuf>;

pub fn walker_paths(root: &Path, extensions: &[&str]) -> Paths {
    sorted_paths(
        Walker::with_options(root, WalkOptions::default())
            .unwrap()
            .filter_map(Result::ok)
            .filter(WalkEntry::is_file)
            .filter(|entry| has_extension(entry.path(), extensions))
            .map(|entry| entry.relative_path().to_path_buf()),
    )
}

pub fn parallel_paths(root: &Path, extensions: &[&str]) -> Paths {
    sorted_paths(
        ParallelWalker::new(root)
            .walk()
            .unwrap()
            .entries
            .into_iter()
            .filter(WalkEntry::is_file)
            .filter(|entry| has_extension(entry.path(), extensions))
            .map(|entry| entry.relative_path().to_path_buf()),
    )
}

pub fn ignore_paths(root: &Path, extensions: &[&str]) -> Paths {
    let mut builder = WalkBuilder::new(root);
    builder.standard_filters(false);
    relative_paths(
        root,
        builder
            .build()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_type().is_some_and(|kind| kind.is_file()))
            .filter(|entry| has_extension(entry.path(), extensions))
            .map(ignore::DirEntry::into_path),
    )
}

pub fn walkdir_paths(root: &Path, extensions: &[&str]) -> Paths {
    relative_paths(
        root,
        WalkDir::new(root)
            .follow_links(false)
            .into_iter()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_type().is_file())
            .filter(|entry| has_extension(entry.path(), extensions))
            .map(walkdir::DirEntry::into_path),
    )
}

pub fn jwalk_paths(root: &Path, extensions: &[&str]) -> Paths {
    relative_paths(
        root,
        JWalkDir::new(root)
            .sort(false)
            .skip_hidden(false)
            .into_iter()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_type().is_file())
            .filter(|entry| has_extension(&entry.path(), extensions))
            .map(|entry| entry.path()),
    )
}

pub fn dirwalk_paths(root: &Path, extensions: &[&str]) -> Paths {
    let result = dirwalk::WalkBuilder::new(root)
        .hidden(true)
        .extensions(extensions)
        .build()
        .unwrap();
    let mut paths = result
        .entries
        .into_iter()
        .filter(|entry| !entry.is_dir)
        .map(|entry| PathBuf::from(entry.relative_path))
        .collect::<Vec<_>>();
    paths.sort_unstable();
    paths
}

/// Minimal standard-library baseline for isolating traversal overhead.
///
/// On Windows `std::fs::read_dir` already uses
/// `FindFirstFileExW(FindExInfoBasic)`/`FindNextFileW` and carries enumeration
/// metadata in each `DirEntry`, so this distinguishes the OS API from
/// higher-level scheduling and allocation costs.
pub fn std_read_dir_paths(root: &Path, extensions: &[&str]) -> Paths {
    let mut directories = vec![root.to_path_buf()];
    let mut paths = Vec::new();
    while let Some(directory) = directories.pop() {
        let Ok(entries) = fs::read_dir(&directory) else {
            continue;
        };
        for entry in entries.filter_map(Result::ok) {
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            let path = entry.path();
            if file_type.is_dir() {
                directories.push(path);
            } else if file_type.is_file() && has_extension(&path, extensions) {
                paths.push(path.strip_prefix(root).unwrap().to_path_buf());
            }
        }
    }
    paths.sort_unstable();
    paths
}

fn relative_paths(root: &Path, paths: impl IntoIterator<Item = PathBuf>) -> Paths {
    sorted_paths(
        paths
            .into_iter()
            .map(|path| path.strip_prefix(root).unwrap().to_path_buf()),
    )
}

fn sorted_paths(paths: impl IntoIterator<Item = PathBuf>) -> Paths {
    let mut paths = paths.into_iter().collect::<Vec<_>>();
    paths.sort_unstable();
    paths
}

fn has_extension(path: &Path, extensions: &[&str]) -> bool {
    path.extension()
        .and_then(|value| value.to_str())
        .is_some_and(|value| extensions.contains(&value.to_ascii_lowercase().as_str()))
}
