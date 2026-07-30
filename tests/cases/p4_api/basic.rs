use crate::support::Fixture;
use std::sync::Arc;
use std::time::Duration;
use weavatrix_scan::{
    ParallelExecutor, ParallelJob, ParallelMultiWalker, ParallelRuntime, ParallelWalker,
    ScanOptions, Scanner, StatefulWalkBuilder, WalkBuilder, WalkControl, WalkOperation,
    WalkOptions, Walker, scan_repository_compact,
};

#[test]
fn low_level_walkers_accept_a_single_file_root() {
    let fixture = Fixture::new("scan-file-root");
    fixture.write("only.rs", "fn only() {}\n");
    let path = fixture.root.join("only.rs");
    let options = WalkOptions::default().with_metadata(true);

    let serial = Walker::with_options(&path, options)
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(serial.len(), 1);
    assert!(serial[0].is_file());
    assert_eq!(serial[0].depth(), 0);
    assert_eq!(serial[0].bytes(), Some(13));
    assert_eq!(serial[0].clone().into_path(), path);

    let parallel = ParallelWalker::new(&path).options(options).walk().unwrap();
    assert_eq!(parallel.entries.len(), 1);
    assert!(parallel.entries[0].is_file());

    let built = WalkBuilder::new(&path)
        .build()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(built.len(), 1);

    let stateful = StatefulWalkBuilder::<(), ()>::new(&path, ())
        .build()
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(stateful.len(), 1);
}

struct RejectingExecutor;

impl ParallelExecutor for RejectingExecutor {
    fn parallelism(&self) -> usize {
        4
    }

    fn try_execute(
        &self,
        _job: ParallelJob,
        _busy_timeout: Option<Duration>,
    ) -> std::io::Result<()> {
        Err(std::io::Error::new(
            std::io::ErrorKind::WouldBlock,
            "external pool busy",
        ))
    }
}

#[test]
fn external_pool_submission_failure_is_typed_and_does_not_hang() {
    let fixture = Fixture::new("scan-external-reject");
    for directory in 0..8 {
        fixture.write(&format!("d{directory:02}/value.rs"), "fn value() {}\n");
    }
    let runtime = ParallelRuntime::external(Arc::new(RejectingExecutor))
        .with_busy_timeout(Some(Duration::from_millis(1)));
    let error = ParallelWalker::new(&fixture.root)
        .with_parallelism(4)
        .runtime(runtime.clone())
        .walk()
        .unwrap_err();
    assert_eq!(error.operation(), WalkOperation::ScheduleWorker);
    assert_eq!(error.io_error().kind(), std::io::ErrorKind::WouldBlock);

    let scanner_error = Scanner::new(&fixture.root)
        .options(
            ScanOptions::default()
                .metadata_only()
                .with_traversal_parallelism(4),
        )
        .runtime(runtime)
        .scan()
        .unwrap_err();
    assert!(scanner_error.to_string().contains("external pool busy"));

    let multi_error = ParallelMultiWalker::new(&fixture.root)
        .add_root(&fixture.root)
        .with_root_parallelism(2)
        .runtime(
            ParallelRuntime::external(Arc::new(RejectingExecutor))
                .with_busy_timeout(Some(Duration::from_millis(1))),
        )
        .visit(|_| WalkControl::Continue)
        .unwrap_err();
    assert_eq!(multi_error.operation(), WalkOperation::ScheduleWorker);
}

#[test]
fn compact_manifest_matches_full_manifest_without_absolute_path_duplication() {
    let fixture = Fixture::new("scan-compact-report");
    fixture.write(".gitignore", "ignored/\n");
    fixture.write("src/a.rs", "fn a() {}\n");
    fixture.write("src/b.rs", "fn b() {}\n");
    fixture.write("ignored/no.rs", "fn hidden() {}\n");
    let options = ScanOptions::default()
        .with_extensions(["rs"])
        .metadata_only()
        .selected_files_only()
        .with_traversal_parallelism(4);
    let full = Scanner::new(&fixture.root)
        .options(options.clone())
        .scan()
        .unwrap();
    let compact = Scanner::new(&fixture.root)
        .options(options)
        .scan_compact()
        .unwrap();

    assert_eq!(compact.revision, full.revision);
    assert_eq!(
        compact
            .files
            .iter()
            .map(|file| (file.relative.as_ref(), file.bytes))
            .collect::<Vec<_>>(),
        full.files
            .iter()
            .map(|file| (file.relative.as_str(), file.bytes))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        compact.absolute_path(&compact.files[0]),
        full.files[0].absolute
    );
    assert!(
        std::mem::size_of::<weavatrix_scan::CompactScannedFile>()
            < std::mem::size_of::<weavatrix_scan::ScannedFile>()
    );
    assert!(compact.files.iter().all(|file| file.content.is_none()));

    let rich_options = ScanOptions::default()
        .with_extensions(["rs"])
        .selected_files_only();
    let rich_full = Scanner::new(&fixture.root)
        .options(rich_options.clone())
        .scan()
        .unwrap();
    let rich_compact = Scanner::new(&fixture.root)
        .options(rich_options)
        .scan_compact()
        .unwrap();
    assert_eq!(rich_compact.revision, rich_full.revision);
    assert_eq!(
        rich_compact
            .files
            .iter()
            .map(weavatrix_scan::CompactScannedFile::content_hash)
            .collect::<Vec<_>>(),
        rich_full
            .files
            .iter()
            .map(|file| file.content_hash.as_deref())
            .collect::<Vec<_>>()
    );
    let cache = rich_compact.to_cache();
    assert_eq!(cache.root, rich_compact.root);
    assert_eq!(cache.entries.len(), rich_compact.files.len());
    assert!(
        cache
            .entries
            .iter()
            .all(|entry| entry.content_hash.starts_with("sha256:"))
    );

    let default_compact = scan_repository_compact(&fixture.root).unwrap();
    assert!(!default_compact.files.is_empty());
}
