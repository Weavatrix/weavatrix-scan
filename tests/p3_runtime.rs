#[allow(dead_code)]
#[path = "support/mod.rs"]
mod support;

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use weavatrix_scan::{ParallelWalker, WalkControl, WalkEvent};

#[test]
fn nested_parallel_walk_from_worker_callback_falls_back_without_starvation() {
    let fixture = support::Fixture::new("scan-runtime-nested");
    for directory in 0..32 {
        fixture.write(
            &format!("branch-{directory:02}/file.rs"),
            "fn nested() {}\n",
        );
    }
    let root = fixture.root.clone();
    let started = Arc::new(AtomicBool::new(false));
    let nested_entries = Arc::new(AtomicUsize::new(0));
    let callback_started = Arc::clone(&started);
    let callback_entries = Arc::clone(&nested_entries);
    let report = ParallelWalker::new(&fixture.root)
        .with_parallelism(4)
        .visit(move |event| {
            if matches!(event, WalkEvent::Entry(entry) if entry.depth() > 0)
                && !callback_started.swap(true, Ordering::AcqRel)
            {
                let report = ParallelWalker::new(&root)
                    .with_parallelism(4)
                    .walk()
                    .expect("nested traversal completes");
                callback_entries.store(report.entries.len(), Ordering::Release);
            }
            WalkControl::Continue
        })
        .unwrap();

    assert!(report.visited > 32);
    assert!(started.load(Ordering::Acquire));
    assert_eq!(
        nested_entries.load(Ordering::Acquire),
        usize::try_from(report.visited).unwrap()
    );
}

#[test]
fn worker_callback_panic_propagates_and_pool_remains_usable() {
    let fixture = support::Fixture::new("scan-runtime-panic");
    fixture.write("branch/file.rs", "fn panic_probe() {}\n");

    let panic = catch_unwind(AssertUnwindSafe(|| {
        ParallelWalker::new(&fixture.root)
            .with_parallelism(4)
            .visit(|event| {
                if matches!(event, WalkEvent::Entry(entry) if entry.depth() > 0) {
                    panic!("worker visitor panic");
                }
                WalkControl::Continue
            })
            .unwrap();
    }));
    assert!(panic.is_err());

    let recovered = ParallelWalker::new(&fixture.root)
        .with_parallelism(4)
        .walk()
        .unwrap();
    assert!(
        recovered
            .entries
            .iter()
            .any(weavatrix_scan::WalkEntry::is_file)
    );
}

#[test]
fn nested_parallel_visit_uses_serial_fallback_controls() {
    let fixture = support::Fixture::new("scan-runtime-nested-visit");
    fixture.write("branch-00/skipped.rs", "fn skipped() {}\n");
    fixture.write("branch-01/stop.rs", "fn stop() {}\n");
    let root = fixture.root.clone();
    let started = Arc::new(AtomicBool::new(false));
    let skipped_file_seen = Arc::new(AtomicBool::new(false));
    let nested_quit = Arc::new(AtomicBool::new(false));

    let outer = ParallelWalker::new(&fixture.root)
        .with_parallelism(4)
        .visit({
            let started = Arc::clone(&started);
            let skipped_file_seen = Arc::clone(&skipped_file_seen);
            let nested_quit = Arc::clone(&nested_quit);
            move |event| {
                if matches!(event, WalkEvent::Entry(entry) if entry.depth() > 0)
                    && !started.swap(true, Ordering::AcqRel)
                {
                    let skipped_file_seen = Arc::clone(&skipped_file_seen);
                    let nested = ParallelWalker::new(&root)
                        .with_parallelism(4)
                        .visit(move |nested_event| match nested_event {
                            WalkEvent::Entry(entry)
                                if entry.relative_path() == std::path::Path::new("branch-00") =>
                            {
                                WalkControl::Skip
                            }
                            WalkEvent::Entry(entry)
                                if entry.relative_path()
                                    == std::path::Path::new("branch-00/skipped.rs") =>
                            {
                                skipped_file_seen.store(true, Ordering::Release);
                                WalkControl::Continue
                            }
                            WalkEvent::Entry(entry) if entry.is_file() => WalkControl::Quit,
                            WalkEvent::Entry(_) | WalkEvent::Error(_) => WalkControl::Continue,
                        })
                        .unwrap();
                    nested_quit.store(nested.quit, Ordering::Release);
                }
                WalkControl::Continue
            }
        })
        .unwrap();

    assert!(outer.visited > 0);
    assert!(started.load(Ordering::Acquire));
    assert!(nested_quit.load(Ordering::Acquire));
    assert!(!skipped_file_seen.load(Ordering::Acquire));
}
