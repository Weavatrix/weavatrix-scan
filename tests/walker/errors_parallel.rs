use super::*;

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
