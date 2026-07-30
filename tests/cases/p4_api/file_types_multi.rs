use crate::support::Fixture;
use std::path::Path;
use std::sync::{Arc, Mutex};
use weavatrix_scan::{
    NamedFileTypes, ParallelMultiWalker, ScanOptions, Scanner, WalkControl, WalkEvent,
};

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
