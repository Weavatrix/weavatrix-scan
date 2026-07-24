#[allow(dead_code)]
#[path = "support/mod.rs"]
mod support;

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use support::Fixture;
use weavatrix_scan::{ErrorPolicy, ParallelWalker, WalkControl, WalkEvent, WalkOptions};

#[test]
fn directory_disappearing_between_discovery_and_read_is_local() {
    let fixture = Fixture::new("scan-fault-read-dir");
    fixture.write("vanish/hidden.rs", "fn hidden() {}\n");
    fixture.write("stable/visible.rs", "fn visible() {}\n");
    let removed = Arc::new(AtomicBool::new(false));
    let visitor_removed = Arc::clone(&removed);
    let vanish = fixture.root.join("vanish");
    let report = ParallelWalker::new(&fixture.root)
        .with_parallelism(4)
        .visit(move |event| {
            if matches!(event, WalkEvent::Entry(entry) if entry.path() == vanish)
                && !visitor_removed.swap(true, Ordering::AcqRel)
            {
                std::fs::remove_dir_all(&vanish).unwrap();
            }
            WalkControl::Continue
        })
        .unwrap();

    assert!(removed.load(Ordering::Acquire));
    assert!(
        report
            .errors
            .iter()
            .any(|error| error.path().ends_with("vanish"))
    );
    assert!(!report.quit);
}

#[test]
fn abort_policy_returns_injected_directory_read_error() {
    let fixture = Fixture::new("scan-fault-abort");
    fixture.write("vanish/hidden.rs", "fn hidden() {}\n");
    fixture.write("stable/visible.rs", "fn visible() {}\n");
    let removed = Arc::new(AtomicBool::new(false));
    let visitor_removed = Arc::clone(&removed);
    let vanish = fixture.root.join("vanish");
    let error = ParallelWalker::new(&fixture.root)
        .options(WalkOptions::default().with_error_policy(ErrorPolicy::Abort))
        .with_parallelism(4)
        .visit(move |event| {
            if matches!(event, WalkEvent::Entry(entry) if entry.path() == vanish)
                && !visitor_removed.swap(true, Ordering::AcqRel)
            {
                std::fs::remove_dir_all(&vanish).unwrap();
            }
            WalkControl::Continue
        })
        .unwrap_err();

    assert!(error.path().ends_with("vanish"));
}
