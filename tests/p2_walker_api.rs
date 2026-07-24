#[allow(dead_code)]
mod support;

use std::path::Path;
use support::Fixture;
use weavatrix_scan::{NamedFileTypes, ScanOptions, Scanner, WalkBuilder};

#[test]
fn multi_root_builder_preserves_native_roots_and_relative_paths() {
    let first = Fixture::new("weavatrix-p2-multi-first");
    let second = Fixture::new("weavatrix-p2-multi-second");
    first.write("first.rs", "fn first() {}\n");
    second.write("second.rs", "fn second() {}\n");

    let entries = WalkBuilder::new(&first.root)
        .add_root(&second.root)
        .sort_by_file_name()
        .build()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    let files = entries
        .iter()
        .filter(|entry| entry.is_file())
        .map(|entry| {
            (
                entry.path().parent().unwrap().to_path_buf(),
                entry.relative_path().to_path_buf(),
            )
        })
        .collect::<Vec<_>>();

    assert_eq!(
        files,
        [
            (first.root.clone(), "first.rs".into()),
            (second.root.clone(), "second.rs".into())
        ]
    );
}

#[test]
fn sorting_filtering_and_contents_first_match_builder_contracts() {
    let fixture = Fixture::new("weavatrix-p2-builder");
    fixture.write("b/file.rs", "fn b() {}\n");
    fixture.write("a/file.rs", "fn a() {}\n");
    fixture.write("skip/hidden.rs", "fn hidden() {}\n");

    let sorted = WalkBuilder::new(&fixture.root)
        .sort_by_name(|left, right| right.cmp(left))
        .filter_directories(|entry| entry.file_name() != "skip")
        .build()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    let depth_one = sorted
        .iter()
        .filter(|entry| entry.depth() == 1)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    assert_eq!(depth_one, ["b", "a"]);
    assert!(
        sorted
            .iter()
            .all(|entry| !entry.path().ends_with(Path::new("skip/hidden.rs")))
    );

    let contents_first = WalkBuilder::new(&fixture.root)
        .sort_by_file_name()
        .filter_directories(|entry| entry.file_name() != "skip")
        .contents_first(true)
        .build()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    let positions = |suffix: &str| {
        contents_first
            .iter()
            .position(|entry| entry.path().ends_with(suffix))
            .unwrap()
    };
    assert!(positions("a/file.rs") < positions("a"));
    assert!(positions("b/file.rs") < positions("b"));
    assert!(
        contents_first
            .last()
            .unwrap()
            .relative_path()
            .as_os_str()
            .is_empty()
    );
}

#[test]
fn named_file_types_combine_with_direct_extensions() {
    let fixture = Fixture::new("weavatrix-p2-types");
    fixture.write("lib.rs", "fn run() {}\n");
    fixture.write("view.tsx", "export const View = () => null;\n");
    fixture.write("main.go", "package main\n");
    fixture.write("README.md", "# docs\n");
    let types = NamedFileTypes::new()
        .with_type("rust", ["rs"])
        .with_type("web", ["ts", "tsx"])
        .select(["rust", "web"]);
    let report = Scanner::new(&fixture.root)
        .options(
            ScanOptions::default()
                .with_extensions(["md"])
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

    assert_eq!(files, ["README.md", "lib.rs", "view.tsx"]);
}

#[test]
fn custom_sort_can_group_directories_before_files() {
    let fixture = Fixture::new("weavatrix-p2-custom-sort");
    fixture.write("z.rs", "fn z() {}\n");
    fixture.write("a/file.rs", "fn a() {}\n");
    let entries = WalkBuilder::new(&fixture.root)
        .sort_by(|left, right| {
            let left_is_file = left.file_type().is_ok_and(|kind| kind.is_file());
            let right_is_file = right.file_type().is_ok_and(|kind| kind.is_file());
            left_is_file
                .cmp(&right_is_file)
                .then_with(|| left.file_name().cmp(&right.file_name()))
        })
        .build()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();

    assert_eq!(entries[1].file_name(), "a");
}
