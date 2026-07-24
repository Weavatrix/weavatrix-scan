#[allow(dead_code)]
#[path = "support/mod.rs"]
mod support;

use support::Fixture;
use weavatrix_scan::{ScanOptions, Scanner, WatchPlan};

#[test]
fn changed_path_scan_matches_full_scan_without_traversing_unchanged_files() {
    let fixture = Fixture::new("scan-watch-update");
    fixture.write(".gitignore", "*.tmp\n");
    fixture.write("src/a.rs", "fn a() {}\n");
    fixture.write("src/b.rs", "fn b() {}\n");
    fixture.write("src/c.rs", "fn c() {}\n");
    let options = ScanOptions::default().with_extensions(["rs"]);
    let previous = Scanner::new(&fixture.root)
        .options(options.clone())
        .scan()
        .unwrap();

    fixture.write("src/b.rs", "fn b_changed() {}\n");
    std::fs::remove_file(fixture.root.join("src/c.rs")).unwrap();
    fixture.write("src/d.rs", "fn d() {}\n");
    fixture.write("src/ignored.tmp", "ignored\n");
    let plan = WatchPlan {
        changed: vec![
            "src/b.rs".to_owned(),
            "src/d.rs".to_owned(),
            "src/ignored.tmp".to_owned(),
        ],
        removed: vec!["src/c.rs".to_owned()],
        full_rescan: false,
        rejected_events: 0,
    };

    let updated = Scanner::new(&fixture.root)
        .options(options.clone())
        .scan_watch_plan(&previous, &plan)
        .unwrap();
    let full = Scanner::new(&fixture.root).options(options).scan().unwrap();

    assert_eq!(updated.files, full.files);
    assert_eq!(updated.skipped, full.skipped);
    assert_eq!(updated.revision, full.revision);
    assert_eq!(updated.cache.content_reads, 2);
    assert_eq!(updated.cache.reused_hashes, 0);
}

#[test]
fn structural_or_unsafe_watch_plans_fall_back_to_complete_scan() {
    let fixture = Fixture::new("scan-watch-fallback");
    fixture.write("a.rs", "fn a() {}\n");
    let options = ScanOptions::default().with_extensions(["rs"]);
    let previous = Scanner::new(&fixture.root)
        .options(options.clone())
        .scan()
        .unwrap();
    fixture.write("new/value.rs", "fn value() {}\n");

    let structural = WatchPlan {
        changed: vec!["new".to_owned()],
        removed: Vec::new(),
        full_rescan: false,
        rejected_events: 0,
    };
    let updated = Scanner::new(&fixture.root)
        .options(options.clone())
        .scan_watch_plan(&previous, &structural)
        .unwrap();
    let full = Scanner::new(&fixture.root)
        .options(options.clone())
        .scan()
        .unwrap();
    assert_eq!(updated.revision, full.revision);

    let unsafe_plan = WatchPlan {
        changed: vec!["../outside.rs".to_owned()],
        removed: Vec::new(),
        full_rescan: false,
        rejected_events: 1,
    };
    let safe = Scanner::new(&fixture.root)
        .options(options)
        .scan_watch_plan(&updated, &unsafe_plan)
        .unwrap();
    assert_eq!(safe.revision, full.revision);
}
