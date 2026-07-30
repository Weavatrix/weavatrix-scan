// Parallel execution quality cases.
use super::support::Fixture;
use std::path::Path;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};
use weavatrix_scan::{
    CancellationToken, ErrorPolicy, ParallelWalker, WalkControl, WalkEvent, WalkOptions,
};

#[test]
fn parallel_visit_streams_prunes_and_honors_cancellation() {
    let fixture = Fixture::new("weavatrix-p0-parallel-visit");
    fixture.write("keep/a.rs", "fn a() {}\n");
    fixture.write("skip/deep/b.rs", "fn b() {}\n");
    let visited = Arc::new(Mutex::new(Vec::new()));
    let visitor_paths = Arc::clone(&visited);

    let report = ParallelWalker::new(&fixture.root)
        .with_parallelism(4)
        .visit(move |event| match event {
            WalkEvent::Entry(entry) => {
                visitor_paths
                    .lock()
                    .unwrap()
                    .push(entry.relative_path().to_path_buf());
                if entry.relative_path() == Path::new("skip") {
                    WalkControl::Skip
                } else {
                    WalkControl::Continue
                }
            }
            WalkEvent::Error(_) => WalkControl::Continue,
        })
        .unwrap();

    assert!(report.errors.is_empty());
    assert!(!report.quit);
    assert!(
        !visited
            .lock()
            .unwrap()
            .iter()
            .any(|path| path == Path::new("skip/deep/b.rs"))
    );

    let token = CancellationToken::new();
    token.cancel();
    let cancelled = ParallelWalker::new(&fixture.root)
        .visit_with_cancellation(&token, |_| WalkControl::Continue)
        .unwrap();
    assert!(cancelled.cancelled);
    assert_eq!(cancelled.visited, 0);
}

#[test]
fn parallel_visit_covers_quit_same_filesystem_serial_and_error_policies() {
    let fixture = Fixture::new("weavatrix-p0-parallel-branches");
    fixture.write("keep/deep/a.rs", "fn a() {}\n");
    fixture.write("other/b.rs", "fn b() {}\n");

    let quit = ParallelWalker::new(&fixture.root)
        .visit(|event| match event {
            WalkEvent::Entry(entry) if entry.depth() == 1 => WalkControl::Quit,
            _ => WalkControl::Continue,
        })
        .unwrap();
    assert!(quit.quit);

    let same_file_system = Arc::new(AtomicUsize::new(0));
    let same_file_system_count = Arc::clone(&same_file_system);
    let report = ParallelWalker::new(&fixture.root)
        .options(WalkOptions::default().with_same_file_system(true))
        .with_parallelism(4)
        .visit(move |event| {
            if matches!(event, WalkEvent::Entry(_)) {
                same_file_system_count.fetch_add(1, Ordering::Relaxed);
            }
            WalkControl::Continue
        })
        .unwrap();
    assert!(report.errors.is_empty());
    assert_eq!(
        usize::try_from(report.visited).unwrap(),
        same_file_system.load(Ordering::Relaxed)
    );

    let serial_paths = Arc::new(Mutex::new(Vec::new()));
    let serial_paths_visitor = Arc::clone(&serial_paths);
    let serial = ParallelWalker::new(&fixture.root)
        .options(WalkOptions::default().with_follow_links(true))
        .visit(move |event| match event {
            WalkEvent::Entry(entry) => {
                serial_paths_visitor
                    .lock()
                    .unwrap()
                    .push(entry.relative_path().to_path_buf());
                if entry.relative_path() == Path::new("keep") {
                    WalkControl::Skip
                } else {
                    WalkControl::Continue
                }
            }
            WalkEvent::Error(_) => WalkControl::Continue,
        })
        .unwrap();
    assert!(serial.errors.is_empty());
    assert!(
        !serial_paths
            .lock()
            .unwrap()
            .contains(&Path::new("keep/deep/a.rs").to_path_buf())
    );

    fixture.write("vanish/file.rs", "fn vanish() {}\n");
    let vanish = fixture.root.join("vanish");
    let remove_for_continue = vanish.clone();
    let errors_seen = Arc::new(AtomicUsize::new(0));
    let errors_for_continue = Arc::clone(&errors_seen);
    let continued = ParallelWalker::new(&fixture.root)
        .visit(move |event| match event {
            WalkEvent::Entry(entry) if entry.relative_path() == Path::new("vanish") => {
                std::fs::remove_dir_all(&remove_for_continue).unwrap();
                WalkControl::Continue
            }
            WalkEvent::Error(_) => {
                errors_for_continue.fetch_add(1, Ordering::Relaxed);
                WalkControl::Continue
            }
            WalkEvent::Entry(_) => WalkControl::Continue,
        })
        .unwrap();
    assert_eq!(continued.errors.len(), 1);
    assert_eq!(errors_seen.load(Ordering::Relaxed), 1);

    fixture.write("vanish/file.rs", "fn vanish() {}\n");
    let remove_for_abort = vanish;
    let aborted = ParallelWalker::new(&fixture.root)
        .options(WalkOptions::default().with_error_policy(ErrorPolicy::Abort))
        .visit(move |event| match event {
            WalkEvent::Entry(entry) if entry.relative_path() == Path::new("vanish") => {
                std::fs::remove_dir_all(&remove_for_abort).unwrap();
                WalkControl::Continue
            }
            _ => WalkControl::Continue,
        });
    assert!(aborted.is_err());
}
