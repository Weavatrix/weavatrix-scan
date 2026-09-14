#[allow(dead_code)]
#[path = "support/mod.rs"]
mod support;

use std::time::Duration;
use support::{Fixture, relatives};
use weavatrix_scan::{
    Error, FullRescanReason, IgnorePolicy, ScanOptions, ScanSession, ScanTermination, Scanner,
    WatchPlan, WatchUpdateReason,
};

#[test]
fn paged_session_reads_reject_a_replaced_generation() {
    let fixture = Fixture::new("scan-page-generation");
    fixture.write("a.rs", "fn a() {}\n");
    fixture.write("b.rs", "fn b() {}\n");
    fixture.write("c.rs", "fn c() {}\n");
    let options = ScanOptions::default()
        .with_extensions(["rs"])
        .metadata_only();
    let mut session = ScanSession::open(&fixture.root, options).unwrap();
    assert_eq!(session.generation(), 1);
    let first = session.files_page(1, 0, 1).unwrap();
    assert_eq!(first[0].relative, "a.rs");
    std::fs::remove_file(fixture.root.join("a.rs")).unwrap();
    session
        .apply_watch_plan(&WatchPlan {
            removed: vec!["a.rs".into()],
            ..WatchPlan::default()
        })
        .unwrap();
    assert_eq!(session.generation(), 2);
    assert!(matches!(
        session.files_page(1, 1, 2),
        Err(Error::StaleSnapshot {
            expected: 1,
            actual: 2
        })
    ));
    let rest = session
        .files_page(2, 0, 8)
        .unwrap()
        .into_iter()
        .map(|file| file.relative)
        .collect::<Vec<_>>();
    assert_eq!(rest, ["b.rs", "c.rs"]);
}

#[test]
fn nested_ignore_on_an_untouched_directory_keeps_the_incremental_path() {
    let fixture = Fixture::new("scan-nested-ignore");
    fixture.write("a.rs", "fn a() {}\n");
    fixture.write("right/b.rs", "fn b() {}\n");
    fixture.write("right/.gitignore", "# keep\n");
    let options = ScanOptions::default().with_extensions(["rs"]);
    let previous = Scanner::new(&fixture.root)
        .options(options.clone())
        .scan()
        .unwrap();
    fixture.write("a.rs", "fn a_changed() {}\n");
    let update = Scanner::new(&fixture.root)
        .options(options)
        .scan_watch_plan_detailed(
            &previous,
            &WatchPlan {
                changed: vec!["a.rs".into()],
                ..WatchPlan::default()
            },
        )
        .unwrap();
    assert_eq!(update.reason, WatchUpdateReason::Incremental);
    assert_eq!(update.report.cache.content_reads, 1);
    assert_eq!(relatives(&update.report), ["a.rs", "right/b.rs"]);
}

#[test]
fn deleting_a_visited_nested_ignore_file_is_an_ignore_input_change() {
    let fixture = Fixture::new("scan-deleted-nested-ignore");
    fixture.write("right/b.rs", "fn b() {}\n");
    fixture.write("right/.gitignore", "*.rs\n");
    let options = ScanOptions::default().with_extensions(["rs"]);
    let previous = Scanner::new(&fixture.root)
        .options(options.clone())
        .scan()
        .unwrap();
    std::fs::remove_file(fixture.root.join("right/.gitignore")).unwrap();
    let update = Scanner::new(&fixture.root)
        .options(options)
        .scan_watch_plan_detailed(
            &previous,
            &WatchPlan {
                changed: vec!["right/b.rs".into()],
                ..WatchPlan::default()
            },
        )
        .unwrap();
    assert_eq!(
        update.reason,
        WatchUpdateReason::FullRescan(FullRescanReason::IgnoreInputChanged)
    );
    assert_eq!(relatives(&update.report), ["right/b.rs"]);
}

#[test]
fn explicit_ignore_file_order_changes_selection_and_descriptor() {
    let fixture = Fixture::new("scan-explicit-order");
    fixture.write("keep.rs", "fn keep() {}\n");
    fixture.write("exclude.rules", "*.rs\n");
    fixture.write("include.rules", "!*.rs\n");
    let include_then_exclude = ScanOptions::default().with_ignore_policy(
        IgnorePolicy::none()
            .with_explicit_file(fixture.root.join("include.rules"))
            .with_explicit_file(fixture.root.join("exclude.rules")),
    );
    let exclude_then_include = ScanOptions::default().with_ignore_policy(
        IgnorePolicy::none()
            .with_explicit_file(fixture.root.join("exclude.rules"))
            .with_explicit_file(fixture.root.join("include.rules")),
    );
    let excluded = Scanner::new(&fixture.root)
        .options(include_then_exclude)
        .scan()
        .unwrap();
    let included = Scanner::new(&fixture.root)
        .options(exclude_then_include)
        .scan()
        .unwrap();
    assert_ne!(excluded.descriptor, included.descriptor);
    let delta = included.delta_from(&excluded);
    assert!(delta.policy_changed);
}

#[test]
fn overlapping_invalidation_prefixes_keep_sibling_paths() {
    let fixture = Fixture::new("scan-overlap-prefix");
    fixture.write("src/a.rs", "fn a() {}\n");
    fixture.write("src/nested/b.rs", "fn b() {}\n");
    fixture.write("src2/c.rs", "fn c() {}\n");
    let options = ScanOptions::default()
        .with_extensions(["rs"])
        .metadata_only();
    let previous = Scanner::new(&fixture.root)
        .options(options.clone())
        .scan()
        .unwrap();
    std::fs::remove_dir_all(fixture.root.join("src")).unwrap();
    let updated = Scanner::new(&fixture.root)
        .options(options)
        .scan_watch_plan(
            &previous,
            &WatchPlan {
                removed: vec!["src".into(), "src/nested".into(), "src/nested/b.rs".into()],
                ..WatchPlan::default()
            },
        )
        .unwrap();
    assert_eq!(relatives(&updated), ["src2/c.rs"]);
}

#[test]
fn cancel_during_the_only_file_records_cancelled() {
    let fixture = Fixture::new("scan-cancel-only");
    fixture.write("one.rs", vec![b'x'; 8 * 1024 * 1024]);
    let report = cancel_while_scanning(&fixture, &ScanOptions::default().with_extensions(["rs"]));
    assert_eq!(report.termination, Some(ScanTermination::Cancelled));
    assert!(
        !report
            .skipped
            .iter()
            .any(|entry| entry.kind == weavatrix_scan::SkipKind::ConcurrentModification)
    );
}

#[test]
fn cancel_during_the_last_file_records_cancelled() {
    let fixture = Fixture::new("scan-cancel-last");
    fixture.write("first.rs", "fn first() {}\n");
    fixture.write("last.rs", vec![b'x'; 8 * 1024 * 1024]);
    let report = cancel_while_scanning(&fixture, &ScanOptions::default().with_extensions(["rs"]));
    assert_eq!(report.termination, Some(ScanTermination::Cancelled));
    assert!(
        !report
            .skipped
            .iter()
            .any(|entry| entry.kind == weavatrix_scan::SkipKind::ConcurrentModification)
    );
}

fn cancel_while_scanning(fixture: &Fixture, options: &ScanOptions) -> weavatrix_scan::ScanReport {
    for delay_ms in [0_u64, 1, 2, 5, 10, 20, 40] {
        let token = weavatrix_scan::CancellationToken::new();
        let cancel = token.clone();
        let root = fixture.root.clone();
        let options = options.clone().with_cancellation(token);
        let worker = std::thread::spawn(move || Scanner::new(root).options(options).scan());
        std::thread::sleep(Duration::from_millis(delay_ms));
        cancel.cancel();
        let report = worker.join().expect("scan thread").unwrap();
        if report.termination == Some(ScanTermination::Cancelled) {
            return report;
        }
    }
    panic!("could not observe an in-flight cancellation");
}
