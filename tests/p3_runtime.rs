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
