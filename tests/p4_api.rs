#[allow(dead_code)]
#[path = "support/mod.rs"]
mod support;

use std::path::Path;
use std::process::{Command, Stdio};
use support::Fixture;
use weavatrix_scan::{
    NamedFileTypes, ParallelMultiWalker, ParallelWalker, ScanOptions, Scanner, StatefulWalkBuilder,
    WalkBuilder, WalkOptions, Walker,
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
