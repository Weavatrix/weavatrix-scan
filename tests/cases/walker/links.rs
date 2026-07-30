// Link-boundary walker cases.
use super::*;

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
