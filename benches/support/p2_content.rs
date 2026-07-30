use crate::p2_ignore_content::{SnapshotChecks, ignore_content_count, ignore_multi_content_count};
use crate::support::{
    BenchmarkCase, EXTENSIONS, Fixture, IGNORE_AWARE_FILES, measure_group, print_measurement,
};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use weavatrix_scan::{
    ContentFileStatus, ContentValidationPolicy, ContentVisitControl, ContentVisitEvent,
    MultiScanner, ScanOptions, Scanner,
};

pub(crate) fn benchmark_multi_content_visit(first: &Fixture, second: &Fixture) {
    let mut cases = vec![
        BenchmarkCase::new("weavatrix-multi-content-revision", || {
            weavatrix_multi_content_count(&first.root, &second.root, false)
        }),
        BenchmarkCase::new("weavatrix-multi-content-streaming", || {
            weavatrix_multi_content_count(&first.root, &second.root, true)
        }),
        BenchmarkCase::new("ignore-multi-content-verified", || {
            ignore_multi_content_count(&first.root, &second.root, SnapshotChecks::BeforeAndAfter)
        }),
    ];
    let results = measure_group(&mut cases);
    for (case, result) in cases.iter().zip(results) {
        assert_eq!(result.count, IGNORE_AWARE_FILES * 2);
        print_measurement("parallel-multi-content-visit", case.name, &result);
    }
}

pub(crate) fn benchmark_content_visit(fixture: &Fixture) {
    let mut cases = vec![
        BenchmarkCase::new("weavatrix-content-fast", || {
            weavatrix_content_count(&fixture.root, ContentValidationPolicy::Fast, 0, false)
        }),
        BenchmarkCase::new("weavatrix-content-strict", || {
            weavatrix_content_count(&fixture.root, ContentValidationPolicy::Strict, 0, false)
        }),
        BenchmarkCase::new("weavatrix-content-streaming-fast", || {
            weavatrix_content_count(&fixture.root, ContentValidationPolicy::Fast, 0, true)
        }),
        BenchmarkCase::new("weavatrix-content-streaming-strict", || {
            weavatrix_content_count(&fixture.root, ContentValidationPolicy::Strict, 0, true)
        }),
        BenchmarkCase::new("ignore-content-visit", || {
            ignore_content_count(&fixture.root, SnapshotChecks::None)
        }),
        BenchmarkCase::new("ignore-content-open-verified", || {
            ignore_content_count(&fixture.root, SnapshotChecks::Before)
        }),
        BenchmarkCase::new("ignore-content-verified", || {
            ignore_content_count(&fixture.root, SnapshotChecks::BeforeAndAfter)
        }),
    ];
    let results = measure_group(&mut cases);
    for (case, result) in cases.iter().zip(results) {
        assert_eq!(result.count, IGNORE_AWARE_FILES);
        print_measurement("parallel-content-visit", case.name, &result);
    }
}

fn weavatrix_content_count(
    root: &Path,
    validation: ContentValidationPolicy,
    workers: usize,
    streaming: bool,
) -> usize {
    let files = Arc::new(AtomicUsize::new(0));
    let bytes = Arc::new(AtomicU64::new(0));
    let scanner = Scanner::new(root).options(
        ScanOptions::default()
            .with_extensions(EXTENSIONS.iter().copied())
            .selected_files_only()
            .metadata_only()
            .with_content_parallelism(workers)
            .with_content_validation(validation),
    );
    let report = if streaming {
        scanner.visit_content_streaming({
            let files = Arc::clone(&files);
            let bytes = Arc::clone(&bytes);
            move |_| {
                let files = Arc::clone(&files);
                let bytes = Arc::clone(&bytes);
                move |event| count_content_event(&event, &files, &bytes)
            }
        })
    } else {
        scanner.visit_content({
            let files = Arc::clone(&files);
            let bytes = Arc::clone(&bytes);
            move |_| {
                let files = Arc::clone(&files);
                let bytes = Arc::clone(&bytes);
                move |event| count_content_event(&event, &files, &bytes)
            }
        })
    }
    .unwrap();
    assert_eq!(
        usize::try_from(report.completed).unwrap(),
        files.load(Ordering::Relaxed)
    );
    std::hint::black_box(bytes.load(Ordering::Relaxed));
    files.load(Ordering::Relaxed)
}

fn count_content_event(
    event: &ContentVisitEvent<'_>,
    files: &AtomicUsize,
    bytes: &AtomicU64,
) -> ContentVisitControl {
    match event {
        ContentVisitEvent::Chunk { bytes: chunk, .. } => {
            bytes.fetch_add(u64::try_from(chunk.len()).unwrap(), Ordering::Relaxed);
        }
        ContentVisitEvent::FileEnd {
            status: ContentFileStatus::Selected,
            ..
        } => {
            files.fetch_add(1, Ordering::Relaxed);
        }
        ContentVisitEvent::FileStart { .. } | ContentVisitEvent::FileEnd { .. } => {}
    }
    ContentVisitControl::Continue
}

fn weavatrix_multi_content_count(first: &Path, second: &Path, streaming: bool) -> usize {
    let files = Arc::new(AtomicUsize::new(0));
    let bytes = Arc::new(AtomicU64::new(0));
    let scanner = MultiScanner::new(first)
        .add_root(second)
        .with_root_parallelism(2)
        .options(
            ScanOptions::default()
                .with_extensions(EXTENSIONS.iter().copied())
                .selected_files_only()
                .metadata_only()
                .with_content_validation(ContentValidationPolicy::Strict),
        );
    let report = if streaming {
        scanner.visit_content_streaming({
            let files = Arc::clone(&files);
            let bytes = Arc::clone(&bytes);
            move |_, _| {
                let files = Arc::clone(&files);
                let bytes = Arc::clone(&bytes);
                move |event| count_content_event(&event, &files, &bytes)
            }
        })
    } else {
        scanner.visit_content({
            let files = Arc::clone(&files);
            let bytes = Arc::clone(&bytes);
            move |_, _| {
                let files = Arc::clone(&files);
                let bytes = Arc::clone(&bytes);
                move |event| count_content_event(&event, &files, &bytes)
            }
        })
    }
    .unwrap();
    assert_eq!(report.len(), 2);
    std::hint::black_box(bytes.load(Ordering::Relaxed));
    files.load(Ordering::Relaxed)
}
