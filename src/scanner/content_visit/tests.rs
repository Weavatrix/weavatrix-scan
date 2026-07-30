use super::{ContentVisitControl, ParallelRuntime, ScanOptions, Scanner, mpsc};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[test]
fn content_visit_is_reentrant_on_its_runtime() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "weavatrix-content-reentrant-{}-{nonce}",
        std::process::id()
    ));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("value.rs"), "fn value() {}\n").unwrap();
    let runtime = ParallelRuntime::dedicated(1).unwrap();
    let nested_runtime = runtime.clone();
    let (sender, receiver) = mpsc::channel();
    runtime
        .try_execute(move || {
            let result = Scanner::new(&root)
                .options(
                    ScanOptions::default()
                        .with_extensions(["rs"])
                        .selected_files_only()
                        .metadata_only(),
                )
                .runtime(nested_runtime)
                .visit_content(|_| |_| ContentVisitControl::Continue)
                .map(|report| report.completed);
            let _ = std::fs::remove_dir_all(root);
            sender.send(result).unwrap();
        })
        .unwrap();
    assert_eq!(
        receiver
            .recv_timeout(Duration::from_secs(5))
            .unwrap()
            .unwrap(),
        1
    );
}

#[test]
fn revision_visit_retains_a_reusable_manifest() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "weavatrix-content-manifest-{}-{nonce}",
        std::process::id()
    ));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("value.rs"), "fn value() {}\n").unwrap();

    let report = Scanner::new(&root)
        .options(
            ScanOptions::default()
                .with_extensions(["rs"])
                .selected_files_only(),
        )
        .visit_content_manifest(|_| |_| ContentVisitControl::Continue)
        .unwrap();
    assert_eq!(report.files.len(), 1);
    assert_eq!(report.files[0].relative.as_ref(), "value.rs");
    assert!(
        report.files[0]
            .content_hash()
            .is_some_and(|hash| hash.starts_with("sha256:"))
    );

    let materialized = report.into_scan_report();
    assert_eq!(materialized.files.len(), 1);
    assert_eq!(
        materialized.files[0].absolute,
        materialized.root.join("value.rs")
    );
    assert!(materialized.files[0].binary_checked);
    assert!(materialized.files[0].content_hash.is_some());
    std::fs::remove_dir_all(root).unwrap();
}
