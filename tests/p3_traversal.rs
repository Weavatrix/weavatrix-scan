#[allow(dead_code)]
#[path = "support/mod.rs"]
mod support;

use std::collections::HashSet;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use support::Fixture;
use weavatrix_scan::{
    ParallelRuntime, ParallelWalker, StatefulWalkBuilder, StatefulWalkEntry, WalkControl,
    WalkEvent, WalkOptions, WalkSkipReason,
};

fn process_stateful_test_batch(
    depth: usize,
    _path: &Path,
    inherited: &mut usize,
    entries: &mut [Result<StatefulWalkEntry<usize>, weavatrix_scan::WalkError>],
) {
    *inherited += 1;
    entries.sort_by(|left, right| {
        let left = left.as_ref().ok().map(StatefulWalkEntry::path);
        let right = right.as_ref().ok().map(StatefulWalkEntry::path);
        right.cmp(&left)
    });
    for entry in entries.iter_mut().filter_map(|entry| entry.as_mut().ok()) {
        entry.state = *inherited + depth;
    }
}

#[test]
fn read_dir_batch_propagates_directory_state_and_attaches_entry_state() {
    let fixture = Fixture::new("scan-stateful-batch");
    fixture.write("a/child/value.rs", "fn value() {}\n");
    fixture.write("b/hidden.rs", "fn hidden() {}\n");
    fixture.write("drop.txt", "drop\n");

    let entries = StatefulWalkBuilder::<usize, (usize, String)>::new(&fixture.root, 0)
        .process_read_dir(|_depth, _path, inherited, entries| {
            *inherited += 1;
            entries.retain(|item| {
                item.as_ref().map_or(true, |entry| {
                    entry.path().file_name() != Some("drop.txt".as_ref())
                })
            });
            entries.sort_by(|left, right| {
                let left = left.as_ref().ok().map(StatefulWalkEntry::path);
                let right = right.as_ref().ok().map(StatefulWalkEntry::path);
                right.cmp(&left)
            });
            for entry in entries.iter_mut().filter_map(|item| item.as_mut().ok()) {
                entry.state = (
                    *inherited,
                    entry
                        .path()
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned(),
                );
                if entry.path().file_name() == Some("b".as_ref()) {
                    entry.set_read_children(false);
                }
            }
        })
        .build()
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();

    let paths = entries
        .iter()
        .map(|entry| entry.entry().relative_path().to_path_buf())
        .collect::<Vec<_>>();
    assert_eq!(
        paths,
        [
            Path::new("").to_path_buf(),
            Path::new("b").to_path_buf(),
            Path::new("a").to_path_buf(),
            Path::new("a/child").to_path_buf(),
            Path::new("a/child/value.rs").to_path_buf(),
        ]
    );
    assert!(!paths.iter().any(|path| path.ends_with("hidden.rs")));
    assert_eq!(entries[2].state.0, 1);
    assert_eq!(entries[3].state.0, 2);
    assert_eq!(entries[4].state.0, 3);
}

#[test]
fn parallel_stateful_batches_preserve_serial_order_state_and_use_workers() {
    let fixture = Fixture::new("scan-parallel-stateful");
    for directory in 0..16 {
        fixture.write(&format!("d{directory:02}/value.rs"), "fn value() {}\n");
    }

    let serial = StatefulWalkBuilder::<usize, usize>::new(&fixture.root, 0)
        .process_read_dir(|depth, path, inherited, entries| {
            process_stateful_test_batch(depth, path, inherited, entries);
        })
        .build()
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
        .into_iter()
        .map(|entry| (entry.entry().relative_path().to_path_buf(), entry.state))
        .collect::<Vec<_>>();

    let worker_threads = Arc::new(Mutex::new(HashSet::new()));
    let callback_threads = Arc::clone(&worker_threads);
    let parallel = StatefulWalkBuilder::<usize, usize>::new(&fixture.root, 0)
        .with_parallelism(4)
        .runtime(ParallelRuntime::dedicated(4).unwrap())
        .process_read_dir(move |depth, path, inherited, entries| {
            callback_threads
                .lock()
                .unwrap()
                .insert(std::thread::current().id());
            if depth == 1 {
                std::thread::sleep(Duration::from_millis(5));
            }
            process_stateful_test_batch(depth, path, inherited, entries);
        })
        .build_parallel_ordered(2)
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
        .into_iter()
        .map(|entry| (entry.entry().relative_path().to_path_buf(), entry.state))
        .collect::<Vec<_>>();

    assert_eq!(parallel, serial);
    assert!(worker_threads.lock().unwrap().len() >= 2);
}

#[test]
fn stateful_batch_preserves_depth_and_cycle_policies() {
    let fixture = Fixture::new("scan-stateful-depth");
    fixture.write("a/b/c/value.rs", "fn value() {}\n");
    let entries = StatefulWalkBuilder::<(), ()>::new(&fixture.root, ())
        .options(WalkOptions::default().with_max_depth(Some(2)))
        .build()
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();

    assert!(entries.iter().any(|entry| {
        entry.depth() == 2 && entry.entry().skip_reason() == Some(WalkSkipReason::MaxDepth)
    }));
    assert!(!entries.iter().any(StatefulWalkEntry::is_file));
}

#[test]
fn ordered_parallel_pull_is_strict_dfs_and_repeatable() {
    let fixture = Fixture::new("scan-ordered-pull");
    fixture.write("b/z.rs", "fn z() {}\n");
    fixture.write("a/c/x.rs", "fn x() {}\n");
    fixture.write("a/a.rs", "fn a() {}\n");

    let collect = || {
        ParallelWalker::new(&fixture.root)
            .with_parallelism(4)
            .into_iter_ordered_bounded(2)
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
            .into_iter()
            .map(|entry| entry.relative_path().to_path_buf())
            .collect::<Vec<_>>()
    };
    let first = collect();
    let second = collect();

    assert_eq!(first, second);
    assert_eq!(
        first,
        [
            Path::new("").to_path_buf(),
            Path::new("a").to_path_buf(),
            Path::new("a/a.rs").to_path_buf(),
            Path::new("a/c").to_path_buf(),
            Path::new("a/c/x.rs").to_path_buf(),
            Path::new("b").to_path_buf(),
            Path::new("b/z.rs").to_path_buf(),
        ]
    );
}

#[test]
fn dropping_ordered_parallel_pull_cancels_and_joins() {
    let fixture = Fixture::new("scan-ordered-drop");
    for directory in 0..64 {
        fixture.write(&format!("d{directory:03}/value.rs"), "fn value() {}\n");
    }
    let mut iterator = ParallelWalker::new(&fixture.root)
        .with_parallelism(4)
        .into_iter_ordered_bounded(1);
    assert!(iterator.next().is_some());
    drop(iterator);

    let count = ParallelWalker::new(&fixture.root)
        .into_iter_ordered_bounded(8)
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
        .len();
    assert_eq!(count, 129);
}

#[test]
fn fallible_parallel_pull_starts_both_ordering_modes() {
    let fixture = Fixture::new("scan-fallible-pull");
    fixture.write("a/value.rs", "fn value() {}\n");

    let unordered = ParallelWalker::new(&fixture.root)
        .try_into_iter_bounded(4)
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    let ordered = ParallelWalker::new(&fixture.root)
        .try_into_iter_ordered_bounded(4)
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();

    assert_eq!(unordered.len(), ordered.len());
    assert_eq!(ordered.len(), 3);
}

#[test]
fn followed_links_remain_parallel_and_cycle_safe() {
    if std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get) < 2 {
        return;
    }
    let fixture = Fixture::new("scan-parallel-follow-links");
    for directory in 0..32 {
        fixture.write(&format!("real/d{directory:03}/value.rs"), "fn value() {}\n");
    }
    if !create_dir_symlink(&fixture.root.join("real"), &fixture.root.join("alias"))
        || !create_dir_symlink(&fixture.root, &fixture.root.join("real/back"))
    {
        return;
    }

    let workers = Arc::new(Mutex::new(HashSet::new()));
    let visitor_workers = Arc::clone(&workers);
    let report = ParallelWalker::new(&fixture.root)
        .options(WalkOptions::default().with_follow_links(true))
        .with_parallelism(4)
        .visit(move |event| {
            if matches!(event, WalkEvent::Entry(entry) if entry.is_file()) {
                visitor_workers
                    .lock()
                    .unwrap()
                    .insert(std::thread::current().id());
                std::thread::sleep(Duration::from_millis(1));
            }
            WalkControl::Continue
        })
        .unwrap();

    assert!(workers.lock().unwrap().len() > 1);
    assert!(report.visited < 200);
    assert!(report.errors.is_empty());
}

#[cfg(unix)]
fn create_dir_symlink(target: &Path, link: &Path) -> bool {
    std::os::unix::fs::symlink(target, link).is_ok()
}

#[cfg(windows)]
fn create_dir_symlink(target: &Path, link: &Path) -> bool {
    std::os::windows::fs::symlink_dir(target, link).is_ok()
}

#[cfg(not(any(unix, windows)))]
fn create_dir_symlink(_target: &Path, _link: &Path) -> bool {
    false
}
