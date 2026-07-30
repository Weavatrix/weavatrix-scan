use crate::support::Fixture;
use std::path::Path;
use std::process::{Command, Stdio};
use weavatrix_scan::{ParallelMultiWalker, ParallelWalker, WalkBuilder, WalkControl, WalkEvent};

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
