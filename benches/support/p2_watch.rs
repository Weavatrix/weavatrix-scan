use crate::p2_common::print_count;
use crate::support::{DIRECTORIES, EXTENSIONS, FILES_PER_LANGUAGE, Fixture, SOURCE_FILES};
use std::path::Path;
use weavatrix_scan::{
    ChangedContentVisitOutcome, ContentVisitControl, ContentVisitEvent, ScanOptions, Scanner,
    WatchEvent, WatchEventKind, WatcherEventAdapter,
};

pub(crate) fn benchmark_watcher_adapter(fixture: &Fixture) {
    const EVENT_COUNT: usize = 1_024;

    let options = ScanOptions::default().with_extensions(EXTENSIONS);
    let report = Scanner::new(&fixture.root)
        .options(options.clone())
        .scan()
        .unwrap();
    let cache = report.to_cache();
    assert_eq!(cache.entries.len(), SOURCE_FILES);
    let adapter = WatcherEventAdapter::with_options(&fixture.root, &options).unwrap();
    let events = watcher_events(&fixture.root, EVENT_COUNT);
    let full_rescan = vec![WatchEvent::new("", WatchEventKind::Rescan)];
    let incremental_plan = adapter.plan(events.clone());
    let changed_plan = adapter.plan(
        events
            .iter()
            .map(|event| WatchEvent::new(event.path.clone(), WatchEventKind::Modify)),
    );
    assert!(!incremental_plan.full_rescan);
    assert_eq!(
        incremental_plan.changed.len() + incremental_plan.removed.len(),
        EVENT_COUNT
    );
    let mut cases = vec![
        crate::support::BenchmarkCase::new("weavatrix-plan", || {
            let plan = adapter.plan(events.clone());
            assert!(!plan.full_rescan);
            plan.changed.len() + plan.removed.len()
        }),
        crate::support::BenchmarkCase::new("weavatrix-cache-apply", || {
            let mut next_cache = cache.clone();
            next_cache.apply_watch_plan(&incremental_plan)
        }),
        crate::support::BenchmarkCase::new("weavatrix-incremental", || {
            let plan = adapter.plan(events.clone());
            let mut next_cache = cache.clone();
            next_cache.apply_watch_plan(&plan)
        }),
        crate::support::BenchmarkCase::new("weavatrix-full-rescan", || {
            let plan = adapter.plan(full_rescan.clone());
            assert!(plan.full_rescan);
            let mut next_cache = cache.clone();
            next_cache.apply_watch_plan(&plan)
        }),
        crate::support::BenchmarkCase::new("weavatrix-changed-scan", || {
            Scanner::new(&fixture.root)
                .options(options.clone())
                .scan_watch_plan(&report, &changed_plan)
                .unwrap()
                .files
                .len()
        }),
        crate::support::BenchmarkCase::new("weavatrix-changed-content-streaming", || {
            let outcome = Scanner::new(&fixture.root)
                .options(options.clone())
                .visit_changed_content_streaming(&changed_plan, |_| {
                    |event| {
                        if let ContentVisitEvent::Chunk { bytes, .. } = event {
                            std::hint::black_box(bytes);
                        }
                        ContentVisitControl::Continue
                    }
                })
                .unwrap();
            let ChangedContentVisitOutcome::Visited(report) = outcome else {
                panic!("file-only benchmark plan unexpectedly required a full rescan");
            };
            usize::try_from(report.content.completed).unwrap()
        }),
        crate::support::BenchmarkCase::new("weavatrix-full-scan", || {
            Scanner::new(&fixture.root)
                .options(options.clone())
                .scan()
                .unwrap()
                .files
                .len()
        }),
    ];
    let results = crate::support::measure_group(&mut cases);
    for (case, result) in cases.iter().zip(results) {
        print_count("watcher-adapter", case.name, "items", &result);
    }
}

fn watcher_events(root: &Path, count: usize) -> Vec<WatchEvent> {
    (0..count)
        .map(|index| {
            let directory = index % DIRECTORIES;
            let file = (index / DIRECTORIES) % FILES_PER_LANGUAGE;
            let path = root.join(format!("module_{directory:03}/file_{file:03}.rs"));
            let kind = if index % 2 == 0 {
                WatchEventKind::Modify
            } else {
                WatchEventKind::Remove
            };
            WatchEvent::new(path, kind)
        })
        .collect()
}
