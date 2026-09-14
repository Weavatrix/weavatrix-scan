#[allow(dead_code)]
#[path = "support/mod.rs"]
mod support;

use std::error::Error as _;
use std::time::Duration;
use support::{Fixture, relatives};
use weavatrix_scan::{
    Error, FullRescanReason, ScanCache, ScanOptions, ScanSession, ScanTermination, Scanner,
    WatchPlan, WatchUpdateReason,
};

#[test]
fn watch_update_reasons_have_stable_labels() {
    let reasons = [
        WatchUpdateReason::Incremental,
        WatchUpdateReason::FullRescan(FullRescanReason::PolicyChanged),
        WatchUpdateReason::FullRescan(FullRescanReason::IgnoreInputChanged),
        WatchUpdateReason::FullRescan(FullRescanReason::StructuralChange),
        WatchUpdateReason::FullRescan(FullRescanReason::IncompletePreviousState),
    ];
    for reason in reasons {
        assert_eq!(reason.to_string(), reason.as_str());
        assert!(
            reason.as_str().starts_with("Incremental")
                || reason.as_str().starts_with("FullRescan:")
        );
    }
}

#[test]
fn compact_manifest_keeps_selected_files_and_termination() {
    let fixture = Fixture::new("scan-compact-contract");
    fixture.write("keep.rs", "fn keep() {}\n");
    let report = Scanner::new(&fixture.root)
        .options(
            ScanOptions::default()
                .with_extensions(["rs"])
                .metadata_only(),
        )
        .scan()
        .unwrap();
    let compact = report.to_compact();
    assert_eq!(compact.files.len(), 1);
    assert_eq!(compact.files[0].relative.as_ref(), "keep.rs");
    assert!(compact.complete);
    assert!(compact.termination.is_none());
    assert_eq!(compact.revision, report.revision);
    let cache = report.to_cache();
    assert!(cache.entries.is_empty());
}

#[test]
fn session_rescan_and_optional_cancel_keep_generation_and_reason() {
    let fixture = Fixture::new("scan-session-rescan");
    fixture.write("a.rs", "fn a() {}\n");
    let options = ScanOptions::default()
        .with_extensions(["rs"])
        .metadata_only();
    let mut session = ScanSession::open(&fixture.root, options).unwrap();
    assert!(session.last_update_reason().is_none());
    session.rescan().unwrap();
    assert_eq!(session.generation(), 2);
    assert_eq!(
        session.last_update_reason(),
        Some(WatchUpdateReason::FullRescan(
            FullRescanReason::StructuralChange
        ))
    );
    let token = weavatrix_scan::CancellationToken::new();
    session
        .apply_watch_plan_with_cancellation(&WatchPlan::default(), Some(token))
        .unwrap();
    assert_eq!(session.generation(), 3);
    assert_eq!(
        session.last_update_reason(),
        Some(WatchUpdateReason::Incremental)
    );
    assert_eq!(session.into_report().files.len(), 1);
}

#[test]
fn stale_snapshot_error_explains_both_generations() {
    let error = Error::stale_snapshot(4, 7);
    assert_eq!(
        error.to_string(),
        "scan snapshot generation 4 is stale; current generation is 7"
    );
    assert!(error.source().is_none());
}

#[test]
fn detailed_watch_plans_report_policy_structure_and_incomplete_fallbacks() {
    let fixture = Fixture::new("scan-fallback-reasons");
    fixture.write("a.rs", "fn a() {}\n");
    let metadata = ScanOptions::default()
        .with_extensions(["rs"])
        .metadata_only();
    let previous = Scanner::new(&fixture.root)
        .options(metadata.clone())
        .scan()
        .unwrap();
    let policy = Scanner::new(&fixture.root)
        .options(ScanOptions::default().with_extensions(["ts"]))
        .scan_watch_plan_detailed(&previous, &WatchPlan::default())
        .unwrap();
    assert_eq!(
        policy.reason,
        WatchUpdateReason::FullRescan(FullRescanReason::PolicyChanged)
    );
    let structural = Scanner::new(&fixture.root)
        .options(metadata.clone())
        .scan_watch_plan_detailed(
            &previous,
            &WatchPlan {
                full_rescan: true,
                ..WatchPlan::default()
            },
        )
        .unwrap();
    assert_eq!(
        structural.reason,
        WatchUpdateReason::FullRescan(FullRescanReason::StructuralChange)
    );
    let token = weavatrix_scan::CancellationToken::new();
    token.cancel();
    let incomplete = Scanner::new(&fixture.root)
        .options(metadata.clone().with_cancellation(token))
        .scan()
        .unwrap();
    let recovered = Scanner::new(&fixture.root)
        .options(metadata)
        .scan_watch_plan_detailed(&incomplete, &WatchPlan::default())
        .unwrap();
    assert_eq!(
        recovered.reason,
        WatchUpdateReason::FullRescan(FullRescanReason::IncompletePreviousState)
    );
}

#[test]
fn overlapping_cache_invalidation_keeps_sibling_prefixes() {
    let fixture = Fixture::new("scan-cache-collapse");
    fixture.write("src/a.rs", "fn a() {}\n");
    fixture.write("src2/b.rs", "fn b() {}\n");
    let report = Scanner::new(&fixture.root)
        .options(ScanOptions::default().with_extensions(["rs"]))
        .scan()
        .unwrap();
    let mut cache = ScanCache::from_report(&report);
    assert_eq!(cache.invalidate(["src", "src/a.rs", "src/nested"]), 1);
    assert_eq!(cache.entries.len(), 1);
    assert_eq!(cache.entries[0].relative, "src2/b.rs");
    let expanded = WatchPlan {
        removed: vec!["src".into(), "src/a.rs".into()],
        ..WatchPlan::default()
    }
    .expand_removed(report.files.iter().map(|file| file.relative.as_str()));
    assert_eq!(expanded, ["src/a.rs"]);
}

#[test]
fn deadline_during_the_only_file_records_timeout() {
    let fixture = Fixture::new("scan-deadline-only");
    fixture.write("one.rs", vec![b'x'; 8 * 1024 * 1024]);
    for timeout_ms in [0_u64, 1, 2, 5] {
        let report = Scanner::new(&fixture.root)
            .options(
                ScanOptions::default()
                    .with_extensions(["rs"])
                    .with_timeout(Some(Duration::from_millis(timeout_ms))),
            )
            .scan()
            .unwrap();
        if report.termination == Some(ScanTermination::Timeout) {
            assert!(
                !report
                    .skipped
                    .iter()
                    .any(|entry| entry.kind == weavatrix_scan::SkipKind::ConcurrentModification)
            );
            return;
        }
    }
    panic!("could not observe an in-flight deadline");
}

#[test]
fn policy_change_makes_an_otherwise_empty_delta_non_empty() {
    let fixture = Fixture::new("scan-policy-delta");
    fixture.write("keep.rs", "fn keep() {}\n");
    let metadata = Scanner::new(&fixture.root)
        .options(
            ScanOptions::default()
                .with_extensions(["rs"])
                .metadata_only(),
        )
        .scan()
        .unwrap();
    let hashed = Scanner::new(&fixture.root)
        .options(ScanOptions::default().with_extensions(["rs"]))
        .scan()
        .unwrap();
    let delta = hashed.delta_from(&metadata);
    assert!(delta.policy_changed);
    assert!(!delta.is_empty());
    assert_eq!(relatives(&hashed), relatives(&metadata));
}
