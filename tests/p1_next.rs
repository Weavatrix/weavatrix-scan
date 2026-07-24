#[allow(dead_code)]
#[path = "support/mod.rs"]
mod support;

use std::cell::RefCell;
use support::Fixture;
use weavatrix_scan::{MultiScanner, NamedFileTypes, ScanOptions, ScanSinkControl, Scanner};

#[test]
fn named_file_types_accept_file_name_and_repository_relative_globs() {
    let fixture = Fixture::new("scan-glob-types");
    fixture.write("Makefile", "all:\n");
    fixture.write("src/lib.rs", "fn lib() {}\n");
    fixture.write("src/generated/out.rs", "fn generated() {}\n");
    fixture.write("web/view.test.ts", "export {};\n");
    fixture.write("web/view.ts", "export {};\n");
    let types = NamedFileTypes::new()
        .with_globs("selected", ["Makefile", "src/**/*.rs", "*.test.ts"])
        .select(["selected"]);

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

    assert_eq!(
        files,
        [
            "Makefile",
            "src/generated/out.rs",
            "src/lib.rs",
            "web/view.test.ts"
        ]
    );
}

#[test]
fn independent_worker_budgets_preserve_selection_and_hashes() {
    let fixture = Fixture::new("scan-worker-budgets");
    for index in 0..300 {
        fixture.write(
            &format!("src/module-{index:03}/lib.rs"),
            format!("pub const VALUE: usize = {index};\n"),
        );
    }
    let serial = Scanner::new(&fixture.root)
        .options(
            ScanOptions::default()
                .with_extensions(["rs"])
                .with_parallelism(1),
        )
        .scan()
        .unwrap();
    let split = Scanner::new(&fixture.root)
        .options(
            ScanOptions::default()
                .with_extensions(["rs"])
                .with_parallelism(1)
                .with_traversal_parallelism(4)
                .with_content_parallelism(2),
        )
        .scan()
        .unwrap();

    assert_eq!(serial.revision, split.revision);
    assert_eq!(serial.files, split.files);
}

#[test]
fn compact_cache_reuses_only_compatible_file_evidence() {
    let fixture = Fixture::new("scan-compact-cache");
    fixture.write("src/lib.rs", "pub fn one() {}\n");
    fixture.write("src/other.rs", "pub fn other() {}\n");
    let options = ScanOptions::default().with_extensions(["rs"]);
    let first = Scanner::new(&fixture.root)
        .options(options.clone())
        .scan()
        .unwrap();
    let cache = first.to_cache();

    assert_eq!(cache.entries.len(), 2);
    assert!(cache.entries.iter().all(|entry| {
        !entry
            .relative
            .contains(&fixture.root.to_string_lossy().into_owned())
    }));
    let unchanged = Scanner::new(&fixture.root)
        .options(options.clone())
        .scan_cached(&cache)
        .unwrap();
    assert_eq!(unchanged.cache.reused_hashes, 2);
    assert_eq!(unchanged.cache.content_reads, 0);

    fixture.write("src/lib.rs", "pub fn changed_and_longer() {}\n");
    let changed = Scanner::new(&fixture.root)
        .options(options.clone())
        .scan_cached(&cache)
        .unwrap();
    assert_eq!(changed.cache.reused_hashes, 1);
    assert_eq!(changed.cache.content_reads, 1);
    assert_ne!(changed.revision, first.revision);

    let mut incompatible = cache;
    incompatible.format_version = incompatible.format_version.saturating_add(1);
    let cold = Scanner::new(&fixture.root)
        .options(options)
        .scan_cached(&incompatible)
        .unwrap();
    assert_eq!(cold.cache.reused_hashes, 0);
    assert_eq!(cold.cache.content_reads, 2);
}

#[test]
fn multi_scanner_runs_roots_in_parallel_but_preserves_input_order() {
    let first = Fixture::new("scan-multi-first");
    let second = Fixture::new("scan-multi-second");
    first.write("first.rs", "fn first() {}\n");
    second.write("second.rs", "fn second() {}\n");

    let report = MultiScanner::new(&second.root)
        .add_root(&first.root)
        .options(
            ScanOptions::default()
                .with_extensions(["rs"])
                .metadata_only(),
        )
        .with_root_parallelism(2)
        .scan()
        .unwrap();

    assert_eq!(report.len(), 2);
    assert_eq!(report.reports[0].root, second.root.canonicalize().unwrap());
    assert_eq!(report.reports[1].root, first.root.canonicalize().unwrap());
    assert_eq!(report.reports[0].files[0].relative, "second.rs");
    assert_eq!(report.reports[1].files[0].relative, "first.rs");
}

#[test]
fn scan_into_is_ordered_and_applies_synchronous_backpressure() {
    let fixture = Fixture::new("scan-into");
    for name in ["c.rs", "a.rs", "b.rs"] {
        fixture.write(name, "fn run() {}\n");
    }
    let emitted = RefCell::new(Vec::new());
    let result = Scanner::new(&fixture.root)
        .options(
            ScanOptions::default()
                .with_extensions(["rs"])
                .metadata_only(),
        )
        .scan_into(|file: &weavatrix_scan::ScannedFile| {
            emitted.borrow_mut().push(file.relative.clone());
            if file.relative == "b.rs" {
                ScanSinkControl::Stop
            } else {
                ScanSinkControl::Continue
            }
        })
        .unwrap();

    assert_eq!(*emitted.borrow(), ["a.rs", "b.rs"]);
    assert_eq!(result.emitted, 2);
    assert!(result.stopped);
    assert_eq!(result.selected, 2);
    assert!(!result.complete);
    assert!(!result.revision.is_empty());

    let options = ScanOptions::default()
        .with_extensions(["rs"])
        .metadata_only();
    let full = Scanner::new(&fixture.root)
        .options(options.clone())
        .scan()
        .unwrap();
    let streamed = Scanner::new(&fixture.root)
        .options(options)
        .scan_into(|_: &weavatrix_scan::ScannedFile| ScanSinkControl::Continue)
        .unwrap();
    assert_eq!(streamed.selected, 3);
    assert!(streamed.complete);
    assert_eq!(streamed.revision, full.revision);
}
