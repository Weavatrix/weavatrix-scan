use crate::support::Fixture;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use weavatrix_scan::{
    CancellationToken, ContentDiscoveryMode, ContentFileStatus, ContentVisitControl,
    ContentVisitEvent, ContentVisitMode, MultiScanner, ScanOptions, Scanner,
};

#[test]
fn streaming_content_visit_omits_manifest_revision_but_keeps_byte_evidence() {
    let fixture = Fixture::new("streaming-content");
    fixture.write("src/a.rs", "fn a() {}\n");
    fixture.write("src/b.rs", "fn b() {}\n");
    let hashes = Arc::new(AtomicUsize::new(0));

    let report = Scanner::new(&fixture.root)
        .options(
            ScanOptions::default()
                .with_extensions(["rs"])
                .selected_files_only()
                .with_content_parallelism(2),
        )
        .visit_content_streaming({
            let hashes = Arc::clone(&hashes);
            move |_| {
                let hashes = Arc::clone(&hashes);
                move |event| {
                    if let ContentVisitEvent::FileEnd {
                        status: ContentFileStatus::Selected,
                        content_hash: Some(_),
                        ..
                    } = event
                    {
                        hashes.fetch_add(1, Ordering::Relaxed);
                    }
                    ContentVisitControl::Continue
                }
            }
        })
        .unwrap();

    assert_eq!(report.mode, ContentVisitMode::Streaming);
    assert!(report.revision.is_empty());
    assert_eq!(report.discovered, 2);
    assert_eq!(report.completed, 2);
    assert_eq!(report.cache.content_reads, 2);
    assert_eq!(hashes.load(Ordering::Relaxed), 2);
}

#[test]
fn buffered_parallel_discovery_preserves_streaming_content_results() {
    let fixture = Fixture::new("buffered-parallel-content");
    fixture.write("src/a.rs", "fn a() {}\n");
    fixture.write("src/nested/b.rs", "fn b() {}\n");
    fixture.write("src/ignored.txt", "not selected\n");

    let visit = |mode| {
        let paths = Arc::new(Mutex::new(Vec::new()));
        let report = Scanner::new(&fixture.root)
            .options(
                ScanOptions::default()
                    .with_extensions(["rs"])
                    .selected_files_only()
                    .with_traversal_parallelism(2)
                    .with_content_parallelism(2)
                    .with_content_discovery(mode),
            )
            .visit_content_streaming({
                let paths = Arc::clone(&paths);
                move |_| {
                    let paths = Arc::clone(&paths);
                    move |event| {
                        if let ContentVisitEvent::FileEnd {
                            file,
                            status: ContentFileStatus::Selected,
                            ..
                        } = event
                        {
                            paths.lock().unwrap().push(file.relative.to_owned());
                        }
                        ContentVisitControl::Continue
                    }
                }
            })
            .unwrap();
        let mut paths = paths.lock().unwrap().clone();
        paths.sort_unstable();
        (paths, report)
    };

    let (streaming_paths, streaming) = visit(ContentDiscoveryMode::Streaming);
    let (parallel_paths, parallel) = visit(ContentDiscoveryMode::BufferedParallel);

    assert_eq!(parallel_paths, streaming_paths);
    assert_eq!(parallel.discovered, streaming.discovered);
    assert_eq!(parallel.completed, streaming.completed);
    assert_eq!(parallel.bytes_emitted, streaming.bytes_emitted);
    assert_eq!(parallel.mode, ContentVisitMode::Streaming);
    assert!(parallel.revision.is_empty());
}

#[test]
fn multi_scanner_quit_cancels_content_across_roots() {
    let first = Fixture::new("multi-content-quit-first");
    let second = Fixture::new("multi-content-quit-second");
    for index in 0..32 {
        first.write(&format!("src/first-{index:02}.rs"), vec![b'a'; 80_000]);
        second.write(&format!("src/second-{index:02}.rs"), vec![b'b'; 80_000]);
    }
    let cancellation = CancellationToken::new();
    let quit = Arc::new(AtomicBool::new(false));

    let report = MultiScanner::new(&first.root)
        .add_root(&second.root)
        .with_root_parallelism(2)
        .options(
            ScanOptions::default()
                .with_extensions(["rs"])
                .selected_files_only()
                .with_content_parallelism(2)
                .with_cancellation(cancellation.clone()),
        )
        .visit_content_streaming({
            let quit = Arc::clone(&quit);
            move |_, _| {
                let quit = Arc::clone(&quit);
                move |_| {
                    if quit.swap(true, Ordering::AcqRel) {
                        ContentVisitControl::Continue
                    } else {
                        ContentVisitControl::Quit
                    }
                }
            }
        })
        .unwrap();

    assert!(cancellation.is_cancelled());
    assert!(report.reports.iter().any(|root| root.stopped));
    assert!(
        report
            .reports
            .iter()
            .map(|root| root.completed)
            .sum::<u64>()
            < 64
    );
}

#[test]
fn legacy_content_visit_report_literal_remains_source_compatible() {
    let report = weavatrix_scan::ContentVisitReport {
        mode: weavatrix_scan::ContentVisitMode::Revision,
        root: std::path::PathBuf::from("."),
        discovered: 0,
        completed: 0,
        opened: 0,
        chunks: 0,
        bytes_read: 0,
        bytes_emitted: 0,
        consumer_skipped: 0,
        stopped: false,
        skipped: Vec::new(),
        warnings: Vec::new(),
        ignore_sources: Vec::new(),
        revision: String::new(),
        complete: true,
        termination: None,
        portable: true,
        cache: weavatrix_scan::ScanCacheStats::default(),
    };

    assert_eq!(report.completed, 0);
}
