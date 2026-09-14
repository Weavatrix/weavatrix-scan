#[allow(dead_code)]
#[path = "support/mod.rs"]
mod support;

use support::{Fixture, relatives, watch_matches};
use weavatrix_scan::{
    IgnorePolicy, ScanOptions, ScanSession, Scanner, StandardSkips, WatchEvent, WatchEventKind,
    WatchPlan, WatcherEventAdapter,
};

#[test]
fn outgoing_directory_rename_matches_full_scan_and_keeps_neighbor_prefix() {
    let fixture = Fixture::new("scan-outgoing-dir");
    fixture.write("src/nested/a.rs", "fn a() {}\n");
    fixture.write("src2/b.rs", "fn b() {}\n");
    let options = ScanOptions::default().with_extensions(["rs"]);
    let previous = Scanner::new(&fixture.root)
        .options(options.clone())
        .scan()
        .unwrap();
    assert_eq!(relatives(&previous), ["src/nested/a.rs", "src2/b.rs"]);
    let moved = fixture
        .root
        .parent()
        .expect("temp parent")
        .join(format!("scan-outgoing-dir-moved-{}", std::process::id()));
    std::fs::rename(fixture.root.join("src"), &moved).unwrap();
    let adapter = WatcherEventAdapter::with_options(&fixture.root, &options).unwrap();
    let plan = adapter.plan([
        WatchEvent::new(fixture.root.join("src"), WatchEventKind::RenameFrom),
        WatchEvent::new(&moved, WatchEventKind::RenameTo),
    ]);
    assert!(plan.rejected_events >= 1);
    assert_eq!(plan.removed, ["src"]);
    assert_eq!(
        relatives(&watch_matches(&fixture, &options, &previous, &plan)),
        ["src2/b.rs"]
    );
    let _ = std::fs::remove_dir_all(moved);
}

#[test]
fn directory_delete_and_file_replace_invalidate_confirmed_subtree() {
    let fixture = Fixture::new("scan-dir-replace");
    fixture.write("src/nested/a.rs", "fn a() {}\n");
    fixture.write("keep.rs", "fn keep() {}\n");
    let options = ScanOptions::default().with_extensions(["rs"]);
    let previous = Scanner::new(&fixture.root)
        .options(options.clone())
        .scan()
        .unwrap();
    std::fs::remove_dir_all(fixture.root.join("src")).unwrap();
    let deleted = watch_matches(
        &fixture,
        &options,
        &previous,
        &WatchPlan {
            removed: vec!["src".into()],
            ..WatchPlan::default()
        },
    );
    fixture.write("src", "fn now_a_file() {}\n");
    watch_matches(
        &fixture,
        &options,
        &deleted,
        &WatchPlan {
            changed: vec!["src".into()],
            ..WatchPlan::default()
        },
    );
}

#[test]
fn paired_and_source_only_renames_follow_the_watch_contract() {
    let fixture = Fixture::new("scan-paired-rename");
    fixture.write("old.rs", "fn old() {}\n");
    fixture.write("stable.rs", "fn stable() {}\n");
    let options = ScanOptions::default().with_extensions(["rs"]);
    let previous = Scanner::new(&fixture.root)
        .options(options.clone())
        .scan()
        .unwrap();
    std::fs::rename(fixture.root.join("old.rs"), fixture.root.join("new.rs")).unwrap();
    let adapter = WatcherEventAdapter::with_options(&fixture.root, &options).unwrap();
    let paired = adapter.plan([
        WatchEvent::new(fixture.root.join("old.rs"), WatchEventKind::RenameFrom),
        WatchEvent::new(fixture.root.join("new.rs"), WatchEventKind::RenameTo),
    ]);
    assert_eq!(paired.removed, ["old.rs"]);
    assert_eq!(paired.changed, ["new.rs"]);
    watch_matches(&fixture, &options, &previous, &paired);

    let incoming = Fixture::new("scan-incoming-dir");
    incoming.write("keep.rs", "fn keep() {}\n");
    let previous = Scanner::new(&incoming.root)
        .options(options.clone())
        .scan()
        .unwrap();
    incoming.write("lib/value.rs", "fn value() {}\n");
    watch_matches(
        &incoming,
        &options,
        &previous,
        &WatchPlan {
            changed: vec!["lib".into()],
            ..WatchPlan::default()
        },
    );
}

#[test]
fn empty_watch_plan_does_not_reuse_a_changed_policy() {
    let fixture = Fixture::new("scan-policy-bind");
    fixture.write("a.rs", "fn a() {}\n");
    fixture.write("b.ts", "export const b = 1\n");
    let previous = Scanner::new(&fixture.root)
        .options(ScanOptions::default().with_extensions(["rs"]))
        .scan()
        .unwrap();
    let empty = WatchPlan::default();
    let typescript = ScanOptions::default().with_extensions(["ts"]);
    let updated = Scanner::new(&fixture.root)
        .options(typescript.clone())
        .scan_watch_plan(&previous, &empty)
        .unwrap();
    let full = Scanner::new(&fixture.root)
        .options(typescript)
        .scan()
        .unwrap();
    assert_eq!(relatives(&updated), ["b.ts"]);
    assert_eq!(updated.files, full.files);
    assert_eq!(updated.revision, full.revision);
    let hashed = Scanner::new(&fixture.root)
        .options(ScanOptions::default().with_extensions(["rs"]))
        .scan_watch_plan(
            &Scanner::new(&fixture.root)
                .options(
                    ScanOptions::default()
                        .with_extensions(["rs"])
                        .metadata_only(),
                )
                .scan()
                .unwrap(),
            &empty,
        )
        .unwrap();
    assert!(hashed.files.iter().all(|file| file.content_hash.is_some()));
}

#[test]
fn ignore_and_selection_policy_changes_force_rescan() {
    let fixture = Fixture::new("scan-ignore-policy");
    fixture.write(".gitignore", "*.tmp\n");
    fixture.write("keep.rs", "fn keep() {}\n");
    fixture.write("skip.tmp", "tmp\n");
    let previous = Scanner::new(&fixture.root)
        .options(ScanOptions::default().with_extensions(["rs", "tmp"]))
        .scan()
        .unwrap();
    let none = ScanOptions::default()
        .with_extensions(["rs", "tmp"])
        .with_ignore_policy(IgnorePolicy::none())
        .with_standard_skips(StandardSkips::Disabled);
    let updated = Scanner::new(&fixture.root)
        .options(none.clone())
        .scan_watch_plan(&previous, &WatchPlan::default())
        .unwrap();
    let full = Scanner::new(&fixture.root).options(none).scan().unwrap();
    assert_eq!(updated.revision, full.revision);
    assert!(relatives(&updated).contains(&"skip.tmp"));
}

#[test]
fn revision_tracks_size_not_completeness_and_policy_is_separate() {
    let fixture = Fixture::new("scan-revision-identity");
    fixture.write("a.rs", "fn a() {}\n");
    let metadata = ScanOptions::default()
        .with_extensions(["rs"])
        .metadata_only();
    let first = Scanner::new(&fixture.root)
        .options(metadata.clone())
        .scan()
        .unwrap();
    fixture.write("a.rs", "fn a_longer() {}\n");
    let resized = Scanner::new(&fixture.root)
        .options(metadata.clone())
        .scan()
        .unwrap();
    assert_ne!(first.revision, resized.revision);
    assert_eq!(first.descriptor, resized.descriptor);
    let hashed = Scanner::new(&fixture.root)
        .options(ScanOptions::default().with_extensions(["rs"]))
        .scan()
        .unwrap();
    assert_ne!(resized.descriptor, hashed.descriptor);
    assert_eq!(relatives(&resized), relatives(&hashed));
    let token = weavatrix_scan::CancellationToken::new();
    token.cancel();
    let cancelled = Scanner::new(&fixture.root)
        .options(metadata.with_cancellation(token))
        .scan()
        .unwrap();
    assert!(!cancelled.complete);
}

#[test]
fn lost_events_require_explicit_resync_not_guessing() {
    let fixture = Fixture::new("scan-lost-events");
    fixture.write("a.rs", "fn a() {}\n");
    let options = ScanOptions::default().with_extensions(["rs"]);
    let previous = Scanner::new(&fixture.root)
        .options(options.clone())
        .scan()
        .unwrap();
    fixture.write("hidden.rs", "fn hidden() {}\n");
    let stale = Scanner::new(&fixture.root)
        .options(options.clone())
        .scan_watch_plan(&previous, &WatchPlan::default())
        .unwrap();
    assert_eq!(relatives(&stale), ["a.rs"]);
    let session = ScanSession::open(&fixture.root, options.clone()).unwrap();
    assert_eq!(relatives(session.report()), ["a.rs", "hidden.rs"]);
    watch_matches(
        &fixture,
        &options,
        &stale,
        &WatchPlan {
            full_rescan: true,
            ..WatchPlan::default()
        },
    );
}

#[test]
fn file_only_update_keeps_the_fast_path() {
    let fixture = Fixture::new("scan-fast-path");
    fixture.write("a.rs", "fn a() {}\n");
    fixture.write("b.rs", "fn b() {}\n");
    let options = ScanOptions::default().with_extensions(["rs"]);
    let previous = Scanner::new(&fixture.root)
        .options(options.clone())
        .scan()
        .unwrap();
    fixture.write("b.rs", "fn b_changed() {}\n");
    let updated = Scanner::new(&fixture.root)
        .options(options)
        .scan_watch_plan(
            &previous,
            &WatchPlan {
                changed: vec!["b.rs".into()],
                ..WatchPlan::default()
            },
        )
        .unwrap();
    assert_eq!(updated.cache.content_reads, 1);
    assert_eq!(relatives(&updated), ["a.rs", "b.rs"]);
}
