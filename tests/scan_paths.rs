#[allow(dead_code)]
#[path = "support/mod.rs"]
mod support;

use support::{Fixture, relatives};
use weavatrix_scan::{CancellationToken, ScanOptions, Scanner, scan_repository_paths};

#[test]
fn scan_paths_matches_selected_manifest_relatives() {
    let fixture = Fixture::new("scan-paths-selected");
    fixture.write("src/a.rs", "fn a() {}\n");
    fixture.write("src/b.js", "export const b = 1\n");
    fixture.write("node_modules/ignored.js", "ignored\n");
    fixture.write(".gitignore", "skip.txt\n");
    fixture.write("skip.txt", "nope\n");
    let options = ScanOptions::default()
        .with_extensions(["rs", "js"])
        .metadata_only()
        .selected_files_only();
    let report = Scanner::new(&fixture.root)
        .options(options.clone())
        .scan()
        .unwrap();
    let paths = Scanner::new(&fixture.root)
        .options(options)
        .scan_paths()
        .unwrap();
    assert_eq!(
        paths,
        relatives(&report)
            .into_iter()
            .map(str::to_owned)
            .collect::<Vec<_>>()
    );
    assert_eq!(paths, vec!["src/a.rs".to_owned(), "src/b.js".to_owned()]);
}

#[test]
fn scan_repository_paths_is_sorted_and_skips_standard_directories() {
    let fixture = Fixture::new("scan-paths-sorted");
    fixture.write("z.txt", "z\n");
    fixture.write("a.txt", "a\n");
    fixture.write("node_modules/pkg/index.js", "ignored\n");
    let paths = scan_repository_paths(&fixture.root).unwrap();
    assert_eq!(paths, vec!["a.txt".to_owned(), "z.txt".to_owned()]);
}

#[test]
fn cancelled_scan_paths_returns_interrupted() {
    let fixture = Fixture::new("scan-paths-cancel");
    fixture.write("a.txt", "a\n");
    let token = CancellationToken::new();
    token.cancel();
    let error = Scanner::new(&fixture.root)
        .options(ScanOptions::default().with_cancellation(token))
        .scan_paths()
        .unwrap_err();
    assert!(error.to_string().contains("cancelled"));
}
