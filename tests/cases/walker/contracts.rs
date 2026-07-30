// Core walker contract cases.
use super::*;

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
        .metadata_only()
        .selected_files_only();
    assert_eq!(scan_options.walk.max_open, 1);
    assert!(scan_options.walk.collect_metadata);
    let report = Scanner::new(&fixture.root)
        .options(scan_options)
        .scan()
        .unwrap();
    assert_eq!(report.files[0].relative, "source.RS");
    assert!(report.skipped.is_empty());

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
