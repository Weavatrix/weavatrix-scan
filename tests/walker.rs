#[allow(dead_code)]
mod support;

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use support::Fixture;
use weavatrix_scan::{
    ErrorPolicy, ParallelWalker, ScanOptions, Scanner, SkipKind, WalkOptions, WalkSkipReason,
    Walker,
};

#[test]
fn public_option_and_entry_contracts_are_exercised() {
    let fixture = Fixture::new("weavatrix-walker-public-contract");
    fixture.write("source.RS", "fn source() {}\n");
    fixture.write("LICENSE", "MIT\n");

    let walk_options = WalkOptions::default()
        .with_max_depth(Some(8))
        .with_max_open(0)
        .with_same_file_system(false)
        .with_follow_links(false)
        .with_metadata(true)
        .with_error_policy(ErrorPolicy::Continue);
    assert_eq!(walk_options.max_depth, Some(8));
    assert_eq!(walk_options.max_open, 1);
    assert!(walk_options.collect_metadata);

    let mut walker = Walker::with_options(&fixture.root, walk_options).unwrap();
    assert_eq!(walker.root(), fixture.root);
    assert_eq!(walker.options(), &walk_options);
    let root = walker.next().unwrap().unwrap();
    assert_eq!(root.relative_path(), Path::new(""));
    assert_eq!(root.depth(), 0);
    assert!(root.is_dir());
    assert!(!root.is_file());
    assert!(!root.is_symlink());
    assert_eq!(root.bytes(), None);
    assert_eq!(root.skip_reason(), None);

    let scan_options = ScanOptions::default()
        .with_max_open(0)
        .with_same_file_system(false)
        .with_follow_links(false)
        .with_error_policy(ErrorPolicy::Continue)
        .with_extensions(["rs", "go", "ts", "js", "py", "c", "cpp", "java", "cs"])
        .metadata_only();
    assert_eq!(scan_options.walk.max_open, 1);
    let report = Scanner::new(&fixture.root)
        .options(scan_options)
        .scan()
        .unwrap();
    assert_eq!(report.files[0].relative, "source.RS");

    let unfiltered = Scanner::new(&fixture.root)
        .options(ScanOptions::default().metadata_only())
        .scan()
        .unwrap();
    assert!(
        unfiltered
            .files
            .iter()
            .any(|file| file.relative == "LICENSE")
    );
}

#[test]
fn parallel_modes_cover_serial_empty_and_same_filesystem_paths() {
    let fixture = Fixture::new("weavatrix-parallel-modes");
    for index in 0..12 {
        fixture.write(&format!("dir-{index}/file.rs"), "fn run() {}\n");
    }

    let same_fs = ParallelWalker::new(&fixture.root)
        .options(
            WalkOptions::default()
                .with_same_file_system(true)
                .with_max_depth(Some(4)),
        )
        .with_parallelism(3)
        .walk()
        .unwrap();
    assert!(same_fs.errors.is_empty());
    assert!(
        same_fs
            .entries
            .iter()
            .any(weavatrix_scan::WalkEntry::is_file)
    );

    let serial = ParallelWalker::new(&fixture.root)
        .options(WalkOptions::default().with_follow_links(true))
        .walk()
        .unwrap();
    assert!(serial.errors.is_empty());

    let empty = Fixture::new("weavatrix-parallel-empty");
    let shallow = ParallelWalker::new(&empty.root)
        .options(WalkOptions::default().with_max_depth(Some(0)))
        .walk()
        .unwrap();
    assert_eq!(shallow.entries.len(), 1);
}

#[test]
fn iterative_walker_handles_deep_trees_and_reports_depth_limits() {
    let fixture = Fixture::new("weavatrix-walker-deep");
    let mut directory = fixture.root.clone();
    for _ in 0..70 {
        directory.push("d");
        fs::create_dir(&directory).unwrap();
    }
    fs::write(directory.join("deep.rs"), "fn deep() {}\n").unwrap();

    let complete = Walker::with_options(&fixture.root, WalkOptions::default().with_max_open(1))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert!(
        complete.iter().any(|entry| {
            entry.file_name() == "deep.rs" && entry.depth() == 71 && entry.is_file()
        })
    );

    let limited = Walker::with_options(
        &fixture.root,
        WalkOptions::default().with_max_depth(Some(12)),
    )
    .unwrap()
    .collect::<Result<Vec<_>, _>>()
    .unwrap();
    assert!(limited.iter().any(|entry| {
        entry.depth() == 12 && entry.skip_reason() == Some(WalkSkipReason::MaxDepth)
    }));
    assert!(!limited.iter().any(|entry| entry.file_name() == "deep.rs"));

    let report = Scanner::new(&fixture.root)
        .options(ScanOptions::default().with_max_depth(Some(12)))
        .scan()
        .unwrap();
    assert!(
        report
            .skipped
            .iter()
            .any(|entry| entry.kind == SkipKind::MaxDepth)
    );
}

#[test]
fn continue_policy_yields_a_local_error_and_keeps_walking() {
    let fixture = Fixture::new("weavatrix-walker-errors");
    fixture.write("keep/file.rs", "fn keep() {}\n");
    fixture.write("vanish/file.rs", "fn vanish() {}\n");

    let mut walker = Walker::with_options(
        &fixture.root,
        WalkOptions::default().with_error_policy(ErrorPolicy::Continue),
    )
    .unwrap();
    let mut visited = Vec::new();
    let local_error = loop {
        let item = walker.next().expect("vanish directory must be visible");
        match item {
            Ok(entry) if entry.relative_path() == Path::new("vanish") => {
                fs::remove_dir_all(entry.path()).unwrap();
                break walker
                    .next()
                    .expect("deleted pending directory must yield an error")
                    .unwrap_err();
            }
            Ok(entry) => visited.push(entry.relative_path().to_path_buf()),
            Err(error) => panic!("unexpected early error: {error}"),
        }
    };
    assert_eq!(local_error.path().file_name().unwrap(), "vanish");
    for item in walker {
        visited.push(item.unwrap().relative_path().to_path_buf());
    }
    assert!(visited.contains(&PathBuf::from("keep/file.rs")));
}

#[test]
fn abort_policy_terminates_after_the_first_local_error() {
    let fixture = Fixture::new("weavatrix-walker-abort");
    fixture.write("vanish/file.rs", "fn vanish() {}\n");
    let mut walker = Walker::with_options(
        &fixture.root,
        WalkOptions::default().with_error_policy(ErrorPolicy::Abort),
    )
    .unwrap();
    loop {
        let entry = walker.next().unwrap().unwrap();
        if entry.relative_path() == Path::new("vanish") {
            fs::remove_dir_all(entry.path()).unwrap();
            break;
        }
    }
    assert!(walker.next().unwrap().is_err());
    assert!(walker.next().is_none());
}

#[test]
fn parallel_walker_matches_serial_paths_on_a_wide_tree() {
    let fixture = Fixture::new("weavatrix-parallel-walker");
    for directory in 0..24 {
        for file in 0..8 {
            fixture.write(
                &format!("module_{directory:02}/file_{file:02}.rs"),
                "fn run() {}\n",
            );
        }
    }
    let options = WalkOptions::default().with_max_open(2);
    let serial = Walker::with_options(&fixture.root, options)
        .unwrap()
        .map(|item| item.unwrap().relative_path().to_path_buf())
        .collect::<BTreeSet<_>>();
    let parallel = ParallelWalker::new(&fixture.root)
        .options(options)
        .with_parallelism(8)
        .walk()
        .unwrap();
    assert!(parallel.errors.is_empty());
    let parallel = parallel
        .entries
        .iter()
        .map(|entry| entry.relative_path().to_path_buf())
        .collect::<BTreeSet<_>>();
    assert_eq!(serial, parallel);
}

#[test]
fn followed_symlink_cycles_are_typed_and_do_not_repeat_the_tree() {
    let fixture = Fixture::new("weavatrix-walker-cycle");
    fixture.write("src/lib.rs", "fn run() {}\n");
    let link = fixture.root.join("src/back");
    if !create_dir_symlink(&fixture.root, &link) {
        return;
    }
    let options = WalkOptions::default().with_follow_links(true);
    let entries = Walker::with_options(&fixture.root, options)
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert!(entries.len() < 10);
    assert!(entries.iter().any(|entry| {
        entry.relative_path() == Path::new("src/back")
            && entry.skip_reason() == Some(WalkSkipReason::SymlinkLoop)
    }));

    let report = Scanner::new(&fixture.root)
        .options(
            ScanOptions::default()
                .with_extensions(["rs"])
                .with_follow_links(true),
        )
        .scan()
        .unwrap();
    assert_eq!(report.files.len(), 1);
    assert!(
        report
            .skipped
            .iter()
            .any(|entry| entry.kind == SkipKind::SymlinkLoop)
    );
}

#[test]
fn followed_sibling_aliases_are_not_misreported_as_cycles() {
    let fixture = Fixture::new("weavatrix-walker-sibling-aliases");
    fixture.write("real/value.rs", "fn value() {}\n");
    let alias = fixture.root.join("alias");
    if !create_dir_symlink(&fixture.root.join("real"), &alias) {
        return;
    }

    let entries = Walker::with_options(
        &fixture.root,
        WalkOptions::default().with_follow_links(true),
    )
    .unwrap()
    .collect::<Result<Vec<_>, _>>()
    .unwrap();
    assert!(
        entries
            .iter()
            .all(|entry| entry.skip_reason() != Some(WalkSkipReason::SymlinkLoop))
    );
    let files = entries
        .iter()
        .filter(|entry| entry.is_file())
        .map(|entry| entry.relative_path().to_path_buf())
        .collect::<BTreeSet<_>>();
    assert_eq!(
        files,
        BTreeSet::from([
            PathBuf::from("alias/value.rs"),
            PathBuf::from("real/value.rs"),
        ])
    );
}

#[test]
fn same_filesystem_walk_accepts_the_root_volume() {
    let fixture = Fixture::new("weavatrix-walker-filesystem");
    fixture.write("src/lib.rs", "fn run() {}\n");
    let entries = Walker::with_options(
        &fixture.root,
        WalkOptions::default().with_same_file_system(true),
    )
    .unwrap()
    .collect::<Result<Vec<_>, _>>()
    .unwrap();
    assert!(
        entries
            .iter()
            .all(|entry| { entry.skip_reason() != Some(WalkSkipReason::FileSystemBoundary) })
    );
    assert!(entries.iter().any(|entry| entry.file_name() == "lib.rs"));
}

#[test]
fn followed_symlinks_cannot_escape_the_root() {
    let fixture = Fixture::new("weavatrix-walker-link-root");
    let outside = Fixture::new("weavatrix-walker-link-outside");
    outside.write("outside.rs", "fn outside() {}\n");
    fixture.write("inside.rs", "fn inside() {}\n");
    let link = fixture.root.join("outside");
    if !create_dir_symlink(&outside.root, &link) {
        return;
    }

    let entries = Walker::with_options(
        &fixture.root,
        WalkOptions::default().with_follow_links(true),
    )
    .unwrap()
    .collect::<Result<Vec<_>, _>>()
    .unwrap();
    assert!(entries.iter().any(|entry| {
        entry.relative_path() == Path::new("outside")
            && entry.skip_reason() == Some(WalkSkipReason::PathEscape)
    }));
    assert!(
        !entries
            .iter()
            .any(|entry| entry.file_name() == "outside.rs")
    );

    let report = Scanner::new(&fixture.root)
        .options(
            ScanOptions::default()
                .with_extensions(["rs"])
                .with_follow_links(true),
        )
        .scan()
        .unwrap();
    assert_eq!(report.files.len(), 1);
    assert!(
        report
            .skipped
            .iter()
            .any(|entry| entry.kind == SkipKind::PathEscape)
    );
}

#[cfg(unix)]
#[test]
fn non_utf8_paths_remain_lossless_and_get_collision_free_manifest_names() {
    use std::ffi::OsString;
    use std::os::unix::ffi::{OsStrExt as _, OsStringExt as _};

    let fixture = Fixture::new("weavatrix-walker-non-utf8");
    let name = OsString::from_vec(vec![b'b', b'a', b'd', 0x80, b'.', b'r', b's']);
    fs::write(fixture.root.join(&name), "fn run() {}\n").unwrap();

    let native = Walker::new(&fixture.root)
        .unwrap()
        .map(Result::unwrap)
        .find(|entry| entry.depth() == 1)
        .unwrap();
    assert_eq!(native.file_name().as_bytes(), name.as_bytes());

    let report = Scanner::new(&fixture.root)
        .options(ScanOptions::default().with_extensions(["rs"]))
        .scan()
        .unwrap();
    assert_eq!(report.files.len(), 1);
    assert_eq!(report.files[0].relative, "bad%80.rs");
}

#[cfg(windows)]
#[test]
fn non_unicode_windows_paths_are_escaped_without_replacement() {
    use std::ffi::OsString;
    use std::os::windows::ffi::{OsStrExt as _, OsStringExt as _};

    let fixture = Fixture::new("weavatrix-walker-non-unicode");
    let name = OsString::from_wide(&[
        u16::from(b'b'),
        u16::from(b'a'),
        u16::from(b'd'),
        0xd800,
        u16::from(b'.'),
        u16::from(b'r'),
        u16::from(b's'),
    ]);
    if fs::write(fixture.root.join(&name), "fn run() {}\n").is_err() {
        return;
    }

    let native = Walker::new(&fixture.root)
        .unwrap()
        .map(Result::unwrap)
        .find(|entry| entry.depth() == 1)
        .unwrap();
    assert_eq!(
        native.file_name().encode_wide().collect::<Vec<_>>(),
        name.encode_wide().collect::<Vec<_>>()
    );

    let report = Scanner::new(&fixture.root)
        .options(ScanOptions::default().with_extensions(["rs"]))
        .scan()
        .unwrap();
    assert_eq!(report.files.len(), 1);
    assert_eq!(report.files[0].relative, "bad%uD800.rs");
}

#[cfg(unix)]
#[test]
fn scanner_continues_after_permission_errors_with_partial_typed_evidence() {
    use std::os::unix::fs::PermissionsExt as _;

    let fixture = Fixture::new("weavatrix-scanner-permission");
    fixture.write("blocked/hidden.rs", "fn hidden() {}\n");
    fixture.write("visible.rs", "fn visible() {}\n");
    let blocked = fixture.root.join("blocked");
    fs::set_permissions(&blocked, fs::Permissions::from_mode(0o000)).unwrap();
    let report = Scanner::new(&fixture.root)
        .options(ScanOptions::default().with_extensions(["rs"]))
        .scan()
        .unwrap();
    fs::set_permissions(&blocked, fs::Permissions::from_mode(0o700)).unwrap();

    if report.complete {
        // Privileged test runners can bypass Unix permission bits.
        return;
    }
    assert_eq!(report.files[0].relative, "visible.rs");
    assert!(
        report
            .skipped
            .iter()
            .any(|entry| entry.kind == SkipKind::IoError)
    );
}

#[cfg(unix)]
fn create_dir_symlink(target: &Path, link: &Path) -> bool {
    std::os::unix::fs::symlink(target, link).is_ok()
}

#[cfg(windows)]
fn create_dir_symlink(target: &Path, link: &Path) -> bool {
    std::os::windows::fs::symlink_dir(target, link).is_ok()
}
