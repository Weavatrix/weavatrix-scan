// Unix-only parity cases.
#![cfg(unix)]

use super::*;

#[cfg(target_os = "linux")]
#[test]
fn non_utf8_paths_and_symlink_loops_have_lossless_differential_evidence() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt as _;

    let fixture = support::Fixture::new("weavatrix-scan-native-parity");
    let native_name = OsString::from_vec(vec![
        b'n', b'a', b't', b'i', b'v', b'e', 0x80, b'.', b'r', b's',
    ]);
    std::fs::write(fixture.root.join(&native_name), "fn native() {}\n").unwrap();
    fixture.write("src/lib.rs", "fn run() {}\n");
    std::os::unix::fs::symlink(&fixture.root, fixture.root.join("src/back")).unwrap();

    let ours = Walker::with_options(
        &fixture.root,
        WalkOptions::default().with_follow_links(true),
    )
    .unwrap()
    .collect::<Result<Vec<_>, _>>()
    .unwrap();
    assert!(ours.iter().any(|entry| {
        entry.relative_path() == Path::new("src/back")
            && entry.skip_reason() == Some(WalkSkipReason::SymlinkLoop)
    }));
    let ours_files = ours
        .iter()
        .filter(|entry| entry.is_file())
        .map(|entry| entry.relative_path().to_path_buf())
        .collect::<BTreeSet<_>>();

    let mut builder = WalkBuilder::new(&fixture.root);
    builder
        .follow_links(true)
        .hidden(false)
        .parents(false)
        .require_git(false);
    let mut ignore_files = BTreeSet::new();
    let mut loop_errors = 0;
    for item in builder.build() {
        match item {
            Ok(entry) if entry.file_type().is_some_and(|kind| kind.is_file()) => {
                ignore_files.insert(
                    entry
                        .path()
                        .strip_prefix(&fixture.root)
                        .unwrap()
                        .to_path_buf(),
                );
            }
            Ok(_) => {}
            Err(_) => loop_errors += 1,
        }
    }
    assert_eq!(ours_files, ignore_files);
    assert!(loop_errors > 0);
}

#[cfg(target_os = "linux")]
#[test]
fn non_utf8_rules_are_lossless_and_percent_rules_match_ignore() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt as _;

    let fixture = support::Fixture::new("weavatrix-scan-native-ignore");
    let native_name = OsString::from_vec(vec![
        b'n', b'a', b't', b'i', b'v', b'e', 0x80, b'.', b'r', b's',
    ]);
    std::fs::write(fixture.root.join(&native_name), "fn native() {}\n").unwrap();
    fixture.write("100%.rs", "fn percent() {}\n");
    fixture.write("visible.rs", "fn visible() {}\n");
    fixture.write(
        ".gitignore",
        [b"native".as_slice(), &[0x80], b".rs\n100%.rs\n".as_slice()].concat(),
    );

    let mut options = ScanOptions::default()
        .with_extensions(["rs"])
        .metadata_only();
    options.standard_skips = StandardSkips::Disabled;
    let ours = Scanner::new(&fixture.root).options(options).scan().unwrap();
    let ours = ours
        .files
        .into_iter()
        .map(|file| file.absolute)
        .collect::<BTreeSet<_>>();

    assert_eq!(ours, BTreeSet::from([fixture.root.join("visible.rs")]));

    fixture.write(".gitignore", "100%.rs\n");
    let mut percent_options = ScanOptions::default()
        .with_extensions(["rs"])
        .metadata_only();
    percent_options.standard_skips = StandardSkips::Disabled;
    let percent_ours = Scanner::new(&fixture.root)
        .options(percent_options)
        .scan()
        .unwrap()
        .files
        .into_iter()
        .map(|file| file.absolute)
        .collect::<BTreeSet<_>>();

    let mut builder = WalkBuilder::new(&fixture.root);
    builder
        .hidden(false)
        .parents(false)
        .require_git(false)
        .git_global(false)
        .git_exclude(false);
    let reference = builder
        .build()
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .path()
                .extension()
                .is_some_and(|extension| extension == "rs")
        })
        .map(ignore::DirEntry::into_path)
        .collect::<BTreeSet<_>>();

    assert_eq!(percent_ours, reference);
}

#[cfg(unix)]
#[test]
fn followed_symlink_loops_are_typed_on_unix() {
    let fixture = support::Fixture::new("weavatrix-scan-loop-parity");
    fixture.write("src/lib.rs", "fn run() {}\n");
    std::os::unix::fs::symlink(&fixture.root, fixture.root.join("src/back")).unwrap();

    let ours = Walker::with_options(
        &fixture.root,
        WalkOptions::default().with_follow_links(true),
    )
    .unwrap()
    .collect::<Result<Vec<_>, _>>()
    .unwrap();
    assert!(ours.iter().any(|entry| {
        entry.relative_path() == Path::new("src/back")
            && entry.skip_reason() == Some(WalkSkipReason::SymlinkLoop)
    }));
}

#[cfg(unix)]
#[test]
fn permission_errors_are_non_fatal_for_both_walkers() {
    use std::os::unix::fs::PermissionsExt as _;

    let fixture = support::Fixture::new("weavatrix-scan-permission-parity");
    fixture.write("blocked/hidden.rs", "fn hidden() {}\n");
    fixture.write("visible.rs", "fn visible() {}\n");
    let blocked = fixture.root.join("blocked");
    std::fs::set_permissions(&blocked, std::fs::Permissions::from_mode(0o000)).unwrap();

    let mut options = ScanOptions::default()
        .with_extensions(["rs"])
        .metadata_only();
    options.standard_skips = StandardSkips::Disabled;
    let ours = Scanner::new(&fixture.root).options(options).scan().unwrap();
    let mut builder = WalkBuilder::new(&fixture.root);
    builder
        .hidden(false)
        .parents(false)
        .require_git(false)
        .git_ignore(false)
        .git_exclude(false)
        .git_global(false);
    let mut ignore_files = BTreeSet::new();
    let mut ignore_errors = 0;
    for item in builder.build() {
        match item {
            Ok(entry) if entry.file_type().is_some_and(|kind| kind.is_file()) => {
                ignore_files.insert(
                    entry
                        .path()
                        .strip_prefix(&fixture.root)
                        .unwrap()
                        .to_string_lossy()
                        .replace('\\', "/"),
                );
            }
            Ok(_) => {}
            Err(_) => ignore_errors += 1,
        }
    }
    std::fs::set_permissions(&blocked, std::fs::Permissions::from_mode(0o700)).unwrap();

    if ours.complete {
        return;
    }
    let ours_files = relative_set(ours.files.iter().map(|file| file.relative.as_str()));
    assert_eq!(ours_files, ignore_files);
    assert!(ignore_errors > 0);
    assert!(
        ours.skipped
            .iter()
            .any(|entry| entry.kind == SkipKind::IoError)
    );
}
