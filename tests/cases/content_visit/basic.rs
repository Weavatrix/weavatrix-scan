use crate::support::Fixture;
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use weavatrix_scan::{
    CancellationToken, ContentFileStatus, ContentVisitControl, ContentVisitEvent, ParallelExecutor,
    ParallelJob, ParallelRuntime, ScanOptions, ScanTermination, Scanner,
};

#[derive(Default)]
struct Observed {
    starts: Vec<(u64, String)>,
    chunks: BTreeMap<String, Vec<u8>>,
    ends: Vec<(u64, String, ContentFileStatus, Option<String>, bool)>,
}

fn record_event(
    observed: &Mutex<Observed>,
    expected_root: &Path,
    event: &ContentVisitEvent<'_>,
) -> ContentVisitControl {
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
            let content = observed.chunks.entry(file.relative.to_owned()).or_default();
            assert_eq!(*offset, u64::try_from(content.len()).unwrap());
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
                *status,
                content_hash.map(str::to_owned),
                *consumer_skipped,
            ));
        }
    }
    ContentVisitControl::Continue
}

#[test]
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
                move |event| record_event(&observed, &expected_root, &event)
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
