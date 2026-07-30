#[allow(dead_code)]
mod support;

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use support::Fixture;
use weavatrix_scan::{
    ErrorPolicy, ParallelWalker, ScanOptions, Scanner, SkipKind, WalkOptions, WalkSkipReason,
    Walker,
};

#[path = "cases/walker/contracts.rs"]
mod contracts;
#[path = "cases/walker/errors_parallel.rs"]
mod errors_parallel;
#[path = "cases/walker/links.rs"]
mod links;
#[path = "cases/walker/native.rs"]
mod native;

#[cfg(unix)]
fn create_dir_symlink(target: &Path, link: &Path) -> bool {
    std::os::unix::fs::symlink(target, link).is_ok()
}

#[cfg(windows)]
fn create_dir_symlink(target: &Path, link: &Path) -> bool {
    std::os::windows::fs::symlink_dir(target, link).is_ok()
}
