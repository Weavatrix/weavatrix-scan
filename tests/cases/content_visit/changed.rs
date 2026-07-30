use crate::support::Fixture;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use weavatrix_scan::{
    ChangedContentVisitOutcome, ContentVisitControl, ContentVisitEvent, ContentVisitMode,
    MultiScanner, ScanOptions, Scanner, SkipKind, WatchPlan,
};

#[test]
fn multi_scanner_content_visit_tags_roots_and_preserves_report_order() {
    let first = Fixture::new("multi-content-first");
    let second = Fixture::new("multi-content-second");
    first.write("src/first.rs", "fn first() {}\n");
    second.write("src/second.rs", "fn second() {}\n");
    let observed = Arc::new(Mutex::new(BTreeMap::<usize, Vec<String>>::new()));
    let options = ScanOptions::default()
        .with_extensions(["rs"])
        .selected_files_only()
        .with_traversal_parallelism(1)
        .with_content_parallelism(2);

    let report = MultiScanner::new(&first.root)
        .add_root(&second.root)
        .options(options.clone())
        .with_root_parallelism(2)
        .visit_content({
            let observed = Arc::clone(&observed);
            move |root_index, _worker_index| {
                let observed = Arc::clone(&observed);
                move |event| {
                    if let ContentVisitEvent::FileStart { file, .. } = event {
                        assert_eq!(file.root_index, root_index);
                        observed
                            .lock()
                            .unwrap()
                            .entry(root_index)
                            .or_default()
                            .push(file.relative.to_owned());
                    }
                    ContentVisitControl::Continue
                }
            }
        })
        .unwrap();

    assert_eq!(report.len(), 2);
    assert_eq!(report.reports[0].root, first.root.canonicalize().unwrap());
    assert_eq!(report.reports[1].root, second.root.canonicalize().unwrap());
    assert_eq!(
        report.reports[0].revision,
        Scanner::new(&first.root)
            .options(options.clone())
            .scan_compact()
            .unwrap()
            .revision
    );
    assert_eq!(
        report.reports[1].revision,
        Scanner::new(&second.root)
            .options(options)
            .scan_compact()
            .unwrap()
            .revision
    );
    let observed = observed.lock().unwrap();
    assert_eq!(observed[&0], ["src/first.rs"]);
    assert_eq!(observed[&1], ["src/second.rs"]);
}

#[test]
fn changed_content_visit_reads_only_safe_changed_paths() {
    let fixture = Fixture::new("changed-content");
    fixture.write("src/changed.rs", "fn changed() {}\n");
    fixture.write("src/untouched.rs", "fn untouched() {}\n");
    let observed = Arc::new(Mutex::new(Vec::new()));
    let plan = WatchPlan {
        changed: vec!["src/changed.rs".to_owned()],
        removed: vec!["src/z-removed.rs".to_owned(), "src/a-removed.rs".to_owned()],
        full_rescan: false,
        rejected_events: 0,
    };

    let outcome = Scanner::new(&fixture.root)
        .options(
            ScanOptions::default()
                .with_extensions(["rs"])
                .selected_files_only()
                .with_content_parallelism(2),
        )
        .visit_changed_content(&plan, {
            let observed = Arc::clone(&observed);
            move |_| {
                let observed = Arc::clone(&observed);
                move |event| {
                    if let ContentVisitEvent::FileStart { file, .. } = event {
                        observed.lock().unwrap().push(file.relative.to_owned());
                    }
                    ContentVisitControl::Continue
                }
            }
        })
        .unwrap();

    let ChangedContentVisitOutcome::Visited(report) = outcome else {
        panic!("safe file-only plan unexpectedly required a full scan");
    };
    assert_eq!(report.content.discovered, 1);
    assert_eq!(report.content.completed, 1);
    assert_eq!(report.content.cache.content_reads, 1);
    assert_eq!(report.removed, ["src/a-removed.rs", "src/z-removed.rs"]);
    assert_eq!(*observed.lock().unwrap(), ["src/changed.rs"]);
}

#[test]
fn changed_content_visit_rejects_structural_plans_before_callbacks() {
    let fixture = Fixture::new("changed-content-structural");
    fixture.write("src/value.rs", "fn value() {}\n");
    let factories = Arc::new(AtomicUsize::new(0));
    let plan = WatchPlan {
        changed: vec!["src".to_owned()],
        removed: Vec::new(),
        full_rescan: false,
        rejected_events: 0,
    };

    let outcome = Scanner::new(&fixture.root)
        .options(ScanOptions::default().with_extensions(["rs"]))
        .visit_changed_content(&plan, {
            let factories = Arc::clone(&factories);
            move |_| {
                factories.fetch_add(1, Ordering::Relaxed);
                |_| ContentVisitControl::Continue
            }
        })
        .unwrap();

    assert!(matches!(
        outcome,
        ChangedContentVisitOutcome::FullRescanRequired
    ));
    assert_eq!(factories.load(Ordering::Relaxed), 0);
}

#[test]
fn changed_streaming_visit_parallelizes_and_filters_changed_paths() {
    let fixture = Fixture::new("changed-content-streaming");
    for index in 0..4 {
        fixture.write(
            &format!("selected-{index}.rs"),
            format!("fn selected_{index}() {{}}\n"),
        );
    }
    fixture.write("large.rs", vec![b'x'; 256]);
    fixture.write("notes.txt", "not selected\n");
    fixture.write("deep/nested/far.rs", "fn too_deep() {}\n");
    fixture.write("target/generated.rs", "fn generated() {}\n");
    let plan = WatchPlan {
        changed: vec![
            "selected-0.rs".to_owned(),
            "selected-1.rs".to_owned(),
            "selected-2.rs".to_owned(),
            "selected-3.rs".to_owned(),
            "large.rs".to_owned(),
            "notes.txt".to_owned(),
            "deep/nested/far.rs".to_owned(),
            "target/generated.rs".to_owned(),
            "missing.rs".to_owned(),
        ],
        removed: vec!["removed.rs".to_owned()],
        full_rescan: false,
        rejected_events: 0,
    };
    let factory_count = Arc::new(AtomicUsize::new(0));
    let starts = Arc::new(Mutex::new(Vec::new()));
    let mut options = ScanOptions::default()
        .with_extensions(["rs"])
        .with_max_depth(Some(2))
        .with_content_parallelism(4);
    options.max_file_bytes = 64;

    let outcome = Scanner::new(&fixture.root)
        .options(options)
        .visit_changed_content_streaming(&plan, {
            let factory_count = Arc::clone(&factory_count);
            let starts = Arc::clone(&starts);
            move |_| {
                factory_count.fetch_add(1, Ordering::Relaxed);
                let starts = Arc::clone(&starts);
                move |event| {
                    if let ContentVisitEvent::FileStart { file, .. } = event {
                        starts.lock().unwrap().push(file.relative.to_owned());
                    }
                    ContentVisitControl::Continue
                }
            }
        })
        .unwrap();

    let ChangedContentVisitOutcome::Visited(report) = outcome else {
        panic!("safe file-only plan unexpectedly required a full scan");
    };
    assert_eq!(report.content.mode, ContentVisitMode::Streaming);
    assert!(report.content.revision.is_empty());
    assert_eq!(report.content.discovered, 4);
    assert_eq!(report.content.completed, 4);
    assert_eq!(report.removed, ["removed.rs"]);
    assert!(factory_count.load(Ordering::Relaxed) > 1);
    let mut starts = starts.lock().unwrap().clone();
    starts.sort_unstable();
    assert_eq!(
        starts,
        [
            "selected-0.rs",
            "selected-1.rs",
            "selected-2.rs",
            "selected-3.rs"
        ]
    );
    for expected in [
        SkipKind::Extension,
        SkipKind::MaxDepth,
        SkipKind::Oversized,
        SkipKind::StandardDirectory,
    ] {
        assert!(
            report
                .content
                .skipped
                .iter()
                .any(|entry| entry.kind == expected),
            "missing {expected:?} in {:?}",
            report
                .content
                .skipped
                .iter()
                .map(|entry| entry.kind)
                .collect::<Vec<_>>()
        );
    }
}
