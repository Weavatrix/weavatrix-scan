mod support;

use ignore::WalkBuilder;
use jwalk::WalkDir as JWalkDir;
use std::collections::BTreeSet;
use std::path::Path;
use support::build_scan_corpus;
use walkdir::WalkDir;
#[cfg(unix)]
use weavatrix_scan::WalkSkipReason;
use weavatrix_scan::{
    ParallelWalker, ScanOptions, Scanner, SkipKind, StandardSkips, WalkOptions, Walker,
};

#[path = "cases/competitor_parity/basics.rs"]
mod basics;
#[path = "cases/competitor_parity/randomized.rs"]
mod randomized;
#[path = "cases/competitor_parity/unix.rs"]
mod unix;

fn ignore_crate_files(root: &Path, extensions: &[&str]) -> BTreeSet<String> {
    let extensions = extensions.iter().copied().collect::<BTreeSet<_>>();
    let mut builder = WalkBuilder::new(root);
    builder
        .add_custom_ignore_filename(".weavatrixignore")
        .git_global(false)
        .git_exclude(false)
        .hidden(false)
        .parents(false)
        .require_git(false);

    builder
        .build()
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .file_type()
                .is_some_and(|file_type| file_type.is_file())
        })
        .filter_map(|entry| {
            let path = entry.into_path();
            let extension = path.extension()?.to_str()?.to_ascii_lowercase();
            extensions.contains(extension.as_str()).then_some(path)
        })
        .filter(|path| !std::fs::read(path).unwrap_or_default().contains(&0))
        .map(|path| {
            path.strip_prefix(root)
                .unwrap()
                .components()
                .map(|component| component.as_os_str().to_string_lossy())
                .collect::<Vec<_>>()
                .join("/")
        })
        .collect()
}

fn randomized_rules(patterns: &[&str], state: &mut u64, count: usize) -> String {
    (0..count)
        .map(|_| {
            *state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            let modulus = u64::try_from(patterns.len()).expect("pattern count fits u64");
            let index = usize::try_from(*state % modulus).expect("remainder fits usize");
            patterns[index]
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn relative_set<'a>(items: impl IntoIterator<Item = &'a str>) -> BTreeSet<String> {
    items.into_iter().map(str::to_owned).collect()
}
