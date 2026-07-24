#[allow(dead_code)]
#[path = "support/mod.rs"]
mod support;

use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use support::Fixture;
use weavatrix_scan::{
    NamedFileTypes, ParallelExecutor, ParallelJob, ParallelMultiWalker, ParallelRuntime,
    ParallelWalker, ScanOptions, Scanner, StatefulWalkBuilder, WalkBuilder, WalkControl, WalkEvent,
    WalkOperation, WalkOptions, Walker, scan_repository_compact,
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

#[test]
fn built_in_file_types_support_composition_and_negation() {
    let fixture = Fixture::new("scan-default-types");
    fixture.write("lib.rs", "fn run() {}\n");
    fixture.write("view.tsx", "export const View = 1;\n");
    fixture.write("legacy.js", "module.exports = 1;\n");
    fixture.write("README.md", "# docs\n");
    let types = NamedFileTypes::defaults()
        .with_composed_type("product", ["rust", "typescript", "javascript"])
        .select(["product"])
        .negate(["javascript"]);
    assert!(types.contains("rust"));
    assert!(types.contains("typescript"));

    let report = Scanner::new(&fixture.root)
        .options(
            ScanOptions::default()
                .with_file_types(types)
                .metadata_only(),
        )
        .scan()
        .unwrap();
    let files = report
        .files
        .iter()
        .map(|file| file.relative.as_str())
        .collect::<Vec<_>>();
    assert_eq!(files, ["lib.rs", "view.tsx"]);

    let non_rust = Scanner::new(&fixture.root)
        .options(
            ScanOptions::default()
                .with_file_types(NamedFileTypes::defaults().negate(["rust"]))
                .metadata_only(),
        )
        .scan()
        .unwrap();
    assert!(non_rust.files.iter().all(|file| {
        !Path::new(&file.relative)
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("rs"))
    }));
}

#[test]
fn built_in_catalog_is_a_strict_superset_of_ignore_defaults() {
    let ours = NamedFileTypes::defaults();
    let mut upstream = ignore::types::TypesBuilder::new();
    upstream.add_defaults();
    let upstream = upstream.definitions();
    let missing = upstream
        .iter()
        .map(ignore::types::FileTypeDef::name)
        .filter(|name| !ours.contains(name))
        .collect::<Vec<_>>();
    assert!(missing.is_empty(), "missing ignore file types: {missing:?}");
    assert!(ours.len() > upstream.len());
    assert!(ours.names().is_sorted());

    let fixture = Fixture::new("scan-expanded-types");
    fixture.write("infra/main.bicep", "resource storage 'x' = {}\n");
    fixture.write(".github/workflows/check.yml", "jobs: {}\n");
    fixture.write("shell/setup.nu", "let ready = true\n");
    fixture.write("api/world.wit", "package example:world;\n");
    fixture.write("src/main.rs", "fn main() {}\n");
    let report = Scanner::new(&fixture.root)
        .options(
            ScanOptions::default()
                .with_file_types(
                    ours.with_composed_type(
                        "modern",
                        ["bicep", "github-actions", "nushell", "wit"],
                    )
                    .select(["modern"]),
                )
                .metadata_only(),
        )
        .scan()
        .unwrap();
    let files = report
        .files
        .iter()
        .map(|file| file.relative.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        files,
        [
            ".github/workflows/check.yml",
            "api/world.wit",
            "infra/main.bicep",
            "shell/setup.nu",
        ]
    );
}

#[test]
fn parallel_raw_multi_root_walk_preserves_root_order() {
    let first = Fixture::new("scan-raw-multi-first");
    let second = Fixture::new("scan-raw-multi-second");
    first.write("first.rs", "fn first() {}\n");
    second.write("second.rs", "fn second() {}\n");

    let report = ParallelMultiWalker::new(&second.root)
        .add_root(&first.root)
        .with_root_parallelism(2)
        .with_traversal_parallelism(2)
        .walk()
        .unwrap();
    assert_eq!(report.len(), 2);
    assert!(
        report.reports[0]
            .entries
            .iter()
            .any(|entry| entry.file_name() == "second.rs")
    );
    assert!(
        report.reports[1]
            .entries
            .iter()
            .any(|entry| entry.file_name() == "first.rs")
    );
}

#[test]
fn parallel_multi_root_streaming_tags_roots_and_preserves_report_order() {
    let first = Fixture::new("scan-stream-multi-first");
    let second = Fixture::new("scan-stream-multi-second");
    first.write("keep/first.rs", "fn first() {}\n");
    first.write("skip/hidden.rs", "fn hidden() {}\n");
    second.write("second.rs", "fn second() {}\n");
    let expected_roots = Arc::new([second.root.clone(), first.root.clone()]);
    let seen = Arc::new(Mutex::new(Vec::new()));
    let callback_seen = Arc::clone(&seen);
    let callback_roots = Arc::clone(&expected_roots);

    let report = ParallelMultiWalker::new(&second.root)
        .add_root(&first.root)
        .with_root_parallelism(2)
        .with_traversal_parallelism(2)
        .visit(move |event| {
            assert_eq!(event.root, callback_roots[event.root_index]);
            match event.event {
                WalkEvent::Entry(entry) if entry.file_name() == "skip" => WalkControl::Skip,
                WalkEvent::Entry(entry) if entry.is_file() => {
                    callback_seen
                        .lock()
                        .unwrap()
                        .push((event.root_index, entry.relative_path().to_path_buf()));
                    WalkControl::Continue
                }
                WalkEvent::Entry(_) | WalkEvent::Error(_) => WalkControl::Continue,
            }
        })
        .unwrap();

    let mut seen = seen.lock().unwrap().clone();
    seen.sort_unstable();
    assert_eq!(
        seen,
        [
            (0, Path::new("second.rs").to_path_buf()),
            (1, Path::new("keep/first.rs").to_path_buf()),
        ]
    );
    assert_eq!(report.len(), 2);
    assert!(report.reports[0].visited >= 2);
    assert!(report.reports[1].visited >= 3);
    assert!(!report.cancelled());
}

#[test]
fn parallel_multi_root_quit_cancels_every_root() {
    let first = Fixture::new("scan-stream-quit-first");
    let second = Fixture::new("scan-stream-quit-second");
    for index in 0..32 {
        first.write(&format!("a{index:02}/value.rs"), "fn value() {}\n");
        second.write(&format!("b{index:02}/value.rs"), "fn value() {}\n");
    }
    let cancellation = weavatrix_scan::CancellationToken::new();
    let stopped = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let callback_stopped = Arc::clone(&stopped);
    let report = ParallelMultiWalker::new(&first.root)
        .add_root(&second.root)
        .with_root_parallelism(2)
        .visit_with_cancellation(&cancellation, move |_| {
            if callback_stopped.swap(true, std::sync::atomic::Ordering::AcqRel) {
                WalkControl::Continue
            } else {
                WalkControl::Quit
            }
        })
        .unwrap();

    assert!(cancellation.is_cancelled());
    assert!(report.quit());
    assert!(report.cancelled());
    assert_eq!(report.len(), 2);
}

#[test]
fn skip_stdout_excludes_redirected_output_file() {
    const CHILD_ROOT: &str = "WEAVATRIX_SKIP_STDOUT_CHILD_ROOT";
    if let Some(root) = std::env::var_os(CHILD_ROOT) {
        let root = std::path::PathBuf::from(root);
        let entries = WalkBuilder::new(&root)
            .skip_stdout(true)
            .sort_by_file_name()
            .build()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert!(
            entries
                .iter()
                .all(|entry| entry.file_name() != "redirected.txt")
        );
        assert!(entries.iter().any(|entry| entry.file_name() == "keep.rs"));

        let parallel = ParallelWalker::new(&root).skip_stdout(true).walk().unwrap();
        assert!(
            parallel
                .entries
                .iter()
                .all(|entry| entry.file_name() != "redirected.txt")
        );

        let redirected_seen = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let visitor_seen = std::sync::Arc::clone(&redirected_seen);
        ParallelWalker::new(&root)
            .skip_stdout(true)
            .visit(move |event| {
                if matches!(
                    event,
                    WalkEvent::Entry(entry) if entry.file_name() == "redirected.txt"
                ) {
                    visitor_seen.store(true, std::sync::atomic::Ordering::Relaxed);
                }
                WalkControl::Continue
            })
            .unwrap();
        assert!(!redirected_seen.load(std::sync::atomic::Ordering::Relaxed));

        for entries in [
            ParallelWalker::new(&root)
                .skip_stdout(true)
                .into_iter_bounded(8)
                .collect::<Result<Vec<_>, _>>()
                .unwrap(),
            ParallelWalker::new(&root)
                .skip_stdout(true)
                .into_iter_ordered_bounded(8)
                .collect::<Result<Vec<_>, _>>()
                .unwrap(),
        ] {
            assert!(
                entries
                    .iter()
                    .all(|entry| entry.file_name() != "redirected.txt")
            );
        }

        let multi = ParallelMultiWalker::new(&root)
            .skip_stdout(true)
            .walk()
            .unwrap();
        assert!(
            multi.reports[0]
                .entries
                .iter()
                .all(|entry| entry.file_name() != "redirected.txt")
        );
        return;
    }

    let fixture = Fixture::new("scan-skip-stdout");
    fixture.write("keep.rs", "fn keep() {}\n");
    let output = std::fs::File::create(fixture.root.join("redirected.txt")).unwrap();
    let status = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "skip_stdout_excludes_redirected_output_file",
            "--nocapture",
        ])
        .env(CHILD_ROOT, &fixture.root)
        .stdout(Stdio::from(output))
        .stderr(Stdio::null())
        .status()
        .unwrap();
    assert!(status.success());
}

#[cfg(feature = "notify")]
#[test]
fn notify_events_map_directly_to_safe_watch_plans() {
    use notify::event::{CreateKind, ModifyKind, RenameMode};
    use notify::{Event, EventKind};
    use weavatrix_scan::WatcherEventAdapter;

    let fixture = Fixture::new("scan-notify-adapter");
    let adapter = WatcherEventAdapter::new(&fixture.root, [".gitignore"]).unwrap();
    let rename = Event::new(EventKind::Modify(ModifyKind::Name(RenameMode::Both)))
        .add_path(fixture.root.join("old.rs"))
        .add_path(fixture.root.join("new.rs"));
    let plan = adapter.plan_notify([rename]);
    assert_eq!(plan.removed, ["old.rs"]);
    assert_eq!(plan.changed, ["new.rs"]);
    assert!(!plan.full_rescan);

    std::fs::create_dir(fixture.root.join("new-directory")).unwrap();
    let directory = Event::new(EventKind::Create(CreateKind::Folder))
        .add_path(fixture.root.join("new-directory"));
    assert!(adapter.plan_notify([directory]).full_rescan);
}

#[test]
fn complete_entry_sorter_can_use_type_and_path() {
    let fixture = Fixture::new("scan-full-entry-sort");
    fixture.write("z.rs", "fn z() {}\n");
    fixture.write("a/value.rs", "fn a() {}\n");
    let entries = WalkBuilder::new(&fixture.root)
        .sort_by(|left, right| {
            let left_file = left.file_type().is_ok_and(|kind| kind.is_file());
            let right_file = right.file_type().is_ok_and(|kind| kind.is_file());
            left_file
                .cmp(&right_file)
                .then_with(|| left.path().cmp(&right.path()))
        })
        .build()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(entries[1].relative_path(), Path::new("a"));
}
