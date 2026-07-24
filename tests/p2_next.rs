#[allow(dead_code)]
#[path = "support/mod.rs"]
mod support;

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};
use support::Fixture;
use weavatrix_scan::{
    ParallelWalker, RootSymlinkPolicy, ScanOptions, Scanner, WalkBuilder, WalkOptions, Walker,
    WatchEvent, WatchEventKind, WatcherEventAdapter,
};

#[test]
fn root_symlink_policy_is_explicit_for_walkers_and_scanners() {
    let target = Fixture::new("scan-root-target");
    let links = Fixture::new("scan-root-links");
    target.write("src/lib.rs", "fn run() {}\n");
    let link = links.root.join("linked-root");
    if !create_directory_symlink(&target.root, &link) {
        return;
    }

    let followed = Walker::new(&link)
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert!(
        followed
            .iter()
            .any(|entry| entry.relative_path().ends_with("src/lib.rs"))
    );
    let rejected = Walker::with_options(
        &link,
        WalkOptions::default().with_root_symlink_policy(RootSymlinkPolicy::Reject),
    )
    .err()
    .unwrap();
    assert_eq!(rejected.io_error().kind(), std::io::ErrorKind::InvalidInput);

    let followed_scan = Scanner::new(&link)
        .options(
            ScanOptions::default()
                .with_extensions(["rs"])
                .metadata_only(),
        )
        .scan()
        .unwrap();
    assert_eq!(followed_scan.files[0].relative, "src/lib.rs");
    let rejected_scan = Scanner::new(&link)
        .options(
            ScanOptions::default()
                .with_extensions(["rs"])
                .with_root_symlink_policy(RootSymlinkPolicy::Reject),
        )
        .scan();
    assert!(rejected_scan.is_err());
}

#[test]
fn parallel_pull_iterator_is_bounded_complete_and_drop_safe() {
    let fixture = Fixture::new("scan-parallel-pull");
    for index in 0..160 {
        fixture.write(&format!("tree/branch-{index:03}/file.rs"), "fn run() {}\n");
    }
    let expected = Walker::new(&fixture.root)
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
        .into_iter()
        .map(|entry| entry.relative_path().to_path_buf())
        .collect::<BTreeSet<_>>();
    let pulled = ParallelWalker::new(&fixture.root)
        .with_parallelism(4)
        .into_iter_bounded(2)
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
        .into_iter()
        .map(|entry| entry.relative_path().to_path_buf())
        .collect::<BTreeSet<_>>();
    assert_eq!(pulled, expected);

    let mut partial = ParallelWalker::new(&fixture.root)
        .with_parallelism(4)
        .into_iter_bounded(1);
    assert!(partial.next().unwrap().is_ok());
    drop(partial);

    let missing = fixture.root.join("missing");
    let first = ParallelWalker::new(missing)
        .into_iter_bounded(1)
        .next()
        .unwrap();
    assert!(first.is_err());
}

#[test]
fn stateful_directory_filter_keeps_one_mutable_state_across_roots() {
    let first = Fixture::new("scan-stateful-first");
    let second = Fixture::new("scan-stateful-second");
    for (fixture, directories) in [(&first, ["a", "b"]), (&second, ["c", "d"])] {
        for directory in directories {
            fixture.write(&format!("{directory}/file.rs"), "fn run() {}\n");
        }
    }
    let calls = Arc::new(Mutex::new(0_usize));
    let callback_calls = Arc::clone(&calls);
    let files = WalkBuilder::new(&first.root)
        .add_root(&second.root)
        .sort_by_file_name()
        .filter_directories_stateful(move |entry| {
            if entry.depth() == 0 {
                return true;
            }
            let mut calls = callback_calls.lock().unwrap();
            *calls += 1;
            *calls <= 2
        })
        .build()
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
        .into_iter()
        .filter(weavatrix_scan::WalkEntry::is_file)
        .map(|entry| entry.file_name().to_owned())
        .collect::<Vec<_>>();

    assert_eq!(files.len(), 2);
    assert_eq!(*calls.lock().unwrap(), 4);
}

#[test]
fn watcher_adapter_coalesces_safe_relative_cache_invalidations() {
    let fixture = Fixture::new("scan-watch-adapter");
    fixture.write("src/a.rs", "fn a() {}\n");
    fixture.write("src/b.rs", "fn b() {}\n");
    let options = ScanOptions::default().with_extensions(["rs"]);
    let report = Scanner::new(&fixture.root)
        .options(options.clone())
        .scan()
        .unwrap();
    let mut cache = report.to_cache();
    let adapter = WatcherEventAdapter::with_options(&fixture.root, &options).unwrap();
    let outside = fixture.root.parent().unwrap().join("outside.rs");
    let plan = adapter.plan([
        WatchEvent::new(fixture.root.join("src/b.rs"), WatchEventKind::Modify),
        WatchEvent::new("src/a.rs", WatchEventKind::Remove),
        WatchEvent::new(outside, WatchEventKind::Modify),
        WatchEvent::new("../escape.rs", WatchEventKind::Modify),
    ]);

    assert_eq!(plan.changed, ["src/b.rs"], "plan={plan:?}");
    assert_eq!(plan.removed, ["src/a.rs"]);
    assert!(!plan.full_rescan);
    assert_eq!(plan.rejected_events, 2);
    assert_eq!(cache.apply_watch_plan(&plan), 2);
    assert!(cache.entries.is_empty());

    let mut cache = report.to_cache();
    let selection_change =
        adapter.plan([WatchEvent::new("nested/.gitignore", WatchEventKind::Modify)]);
    assert!(selection_change.full_rescan);
    assert_eq!(cache.apply_watch_plan(&selection_change), 2);
    assert!(cache.entries.is_empty());
}

#[cfg(unix)]
fn create_directory_symlink(target: &std::path::Path, link: &std::path::Path) -> bool {
    std::os::unix::fs::symlink(target, link).is_ok()
}

#[cfg(windows)]
fn create_directory_symlink(target: &std::path::Path, link: &std::path::Path) -> bool {
    std::os::windows::fs::symlink_dir(target, link).is_ok()
}
