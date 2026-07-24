#[allow(dead_code)]
#[path = "support/mod.rs"]
mod support;

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use support::Fixture;
use weavatrix_scan::{
    CancellationToken, ChangedContentVisitOutcome, ContentFileStatus, ContentVisitControl,
    ContentVisitEvent, ContentVisitMode, MultiScanner, ParallelExecutor, ParallelJob,
    ParallelRuntime, ScanOptions, ScanTermination, Scanner, WatchPlan,
};

#[derive(Default)]
struct Observed {
    starts: Vec<(u64, String)>,
    chunks: BTreeMap<String, Vec<u8>>,
    ends: Vec<(u64, String, ContentFileStatus, Option<String>, bool)>,
}

#[test]
#[allow(clippy::too_many_lines)]
fn parallel_content_visit_streams_once_with_stable_identity_and_revision() {
    let fixture = Fixture::new("scan-content-visit");
    let first = vec![b'a'; 130_000];
    let second = b"fn second() {}\n".to_vec();
    fixture.write("src/a.rs", &first);
    fixture.write("src/b.rs", &second);
    fixture.write("src/binary.rs", [0, 1, 2, 3]);
    fixture.write("src/ignored.txt", "not selected");
    let options = ScanOptions::default()
        .with_extensions(["rs"])
        .selected_files_only()
        .with_traversal_parallelism(1)
        .with_content_parallelism(3);
    let observed = Arc::new(Mutex::new(Observed::default()));
    let factory_count = Arc::new(AtomicUsize::new(0));
    let expected_root = fixture.root.canonicalize().unwrap();

    let report = Scanner::new(&fixture.root)
        .options(options.clone())
        .visit_content({
            let observed = Arc::clone(&observed);
            let factory_count = Arc::clone(&factory_count);
            let expected_root = expected_root.clone();
            move |_| {
                factory_count.fetch_add(1, Ordering::Relaxed);
                let observed = Arc::clone(&observed);
                let expected_root = expected_root.clone();
                move |event| {
                    let mut observed = observed.lock().unwrap();
                    match event {
                        ContentVisitEvent::FileStart { file, .. } => {
                            assert_eq!(file.root, expected_root);
                            assert_eq!(file.root_index, 0);
                            observed
                                .starts
                                .push((file.sequence, file.relative.to_owned()));
                        }
                        ContentVisitEvent::Chunk {
                            file,
                            offset,
                            bytes,
                            ..
                        } => {
                            let content =
                                observed.chunks.entry(file.relative.to_owned()).or_default();
                            assert_eq!(offset, u64::try_from(content.len()).unwrap());
                            content.extend_from_slice(bytes);
                        }
                        ContentVisitEvent::FileEnd {
                            file,
                            status,
                            content_hash,
                            consumer_skipped,
                            ..
                        } => {
                            observed.ends.push((
                                file.sequence,
                                file.relative.to_owned(),
                                status,
                                content_hash.map(str::to_owned),
                                consumer_skipped,
                            ));
                        }
                    }
                    ContentVisitControl::Continue
                }
            }
        })
        .unwrap();

    let manifest = Scanner::new(&fixture.root)
        .options(options)
        .scan_compact()
        .unwrap();
    assert_eq!(report.revision, manifest.revision);
    assert_eq!(report.discovered, 3);
    assert_eq!(report.completed, 2);
    assert_eq!(report.opened, 3);
    assert_eq!(report.cache.content_reads, 3);
    assert_eq!(factory_count.load(Ordering::Relaxed), 3);
    assert!(report.complete);
    assert!(!report.stopped);
    assert_eq!(report.consumer_skipped, 0);

    let mut observed = observed.lock().unwrap();
    let mut sequences = observed
        .starts
        .iter()
        .map(|(sequence, _)| *sequence)
        .collect::<Vec<_>>();
    sequences.sort_unstable();
    assert_eq!(sequences, [0, 1, 2]);
    observed
        .starts
        .sort_unstable_by(|left, right| left.1.cmp(&right.1));
    observed
        .ends
        .sort_unstable_by(|left, right| left.1.cmp(&right.1));
    assert_eq!(
        observed
            .starts
            .iter()
            .map(|(_, relative)| relative.as_str())
            .collect::<Vec<_>>(),
        ["src/a.rs", "src/b.rs", "src/binary.rs",]
    );
    assert_eq!(observed.chunks["src/a.rs"], first);
    assert_eq!(observed.chunks["src/b.rs"], second);
    assert!(!observed.chunks.contains_key("src/binary.rs"));
    assert_eq!(observed.ends[0].2, ContentFileStatus::Selected);
    assert!(
        observed.ends[0]
            .3
            .as_deref()
            .is_some_and(|hash| hash.starts_with("sha256:"))
    );
    assert_eq!(observed.ends[1].2, ContentFileStatus::Selected);
    assert_eq!(observed.ends[2].2, ContentFileStatus::Binary);
}

#[test]
fn skip_file_stops_delivery_without_allocating_the_whole_file() {
    let fixture = Fixture::new("scan-content-skip");
    fixture.write("large.rs", vec![b'x'; 200_000]);
    let saw_end = Arc::new(AtomicBool::new(false));
    let report = Scanner::new(&fixture.root)
        .options(
            ScanOptions::default()
                .with_extensions(["rs"])
                .selected_files_only()
                .metadata_only()
                .with_traversal_parallelism(1),
        )
        .visit_content({
            let saw_end = Arc::clone(&saw_end);
            move |_| {
                let saw_end = Arc::clone(&saw_end);
                move |event| match event {
                    ContentVisitEvent::Chunk { .. } => ContentVisitControl::SkipFile,
                    ContentVisitEvent::FileEnd {
                        consumer_skipped, ..
                    } => {
                        assert!(consumer_skipped);
                        saw_end.store(true, Ordering::Release);
                        ContentVisitControl::Continue
                    }
                    ContentVisitEvent::FileStart { .. } => ContentVisitControl::Continue,
                }
            }
        })
        .unwrap();

    assert_eq!(report.discovered, 1);
    assert_eq!(report.completed, 1);
    assert_eq!(report.consumer_skipped, 1);
    assert_eq!(report.chunks, 1);
    assert_eq!(report.bytes_read, 64 * 1024);
    assert_eq!(report.bytes_emitted, 64 * 1024);
    assert!(saw_end.load(Ordering::Acquire));
}

#[test]
fn quit_cancels_all_content_workers() {
    let fixture = Fixture::new("scan-content-quit");
    for index in 0..32 {
        fixture.write(&format!("src/file-{index:02}.rs"), vec![b'q'; 80_000]);
    }
    let cancellation = CancellationToken::new();
    let stopped = Arc::new(AtomicBool::new(false));
    let report = Scanner::new(&fixture.root)
        .options(
            ScanOptions::default()
                .with_extensions(["rs"])
                .selected_files_only()
                .with_traversal_parallelism(1)
                .with_content_parallelism(4)
                .with_cancellation(cancellation.clone()),
        )
        .visit_content({
            let stopped = Arc::clone(&stopped);
            move |_| {
                let stopped = Arc::clone(&stopped);
                move |_| {
                    if stopped.swap(true, Ordering::AcqRel) {
                        ContentVisitControl::Continue
                    } else {
                        ContentVisitControl::Quit
                    }
                }
            }
        })
        .unwrap();

    assert!(cancellation.is_cancelled());
    assert!(report.stopped);
    assert!(!report.complete);
    assert_eq!(report.termination, Some(ScanTermination::Cancelled));
}

struct RejectingExecutor;

impl ParallelExecutor for RejectingExecutor {
    fn parallelism(&self) -> usize {
        4
    }

    fn try_execute(
        &self,
        _job: ParallelJob,
        _busy_timeout: Option<std::time::Duration>,
    ) -> std::io::Result<()> {
        Err(std::io::Error::new(
            std::io::ErrorKind::WouldBlock,
            "content executor busy",
        ))
    }
}

#[test]
fn content_worker_submission_failure_is_fallible() {
    let fixture = Fixture::new("scan-content-reject");
    fixture.write("a.rs", "fn a() {}\n");
    fixture.write("b.rs", "fn b() {}\n");
    let error = Scanner::new(&fixture.root)
        .options(
            ScanOptions::default()
                .with_extensions(["rs"])
                .selected_files_only()
                .metadata_only()
                .with_traversal_parallelism(1)
                .with_content_parallelism(2),
        )
        .runtime(ParallelRuntime::external(Arc::new(RejectingExecutor)))
        .visit_content(|_| |_| ContentVisitControl::Continue)
        .unwrap_err();
    assert!(error.to_string().contains("content executor busy"));
}

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
