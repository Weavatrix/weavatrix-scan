use std::fs;
use std::hint::black_box;
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use weavatrix_scan::{ScanOptions, Scanner, WatchPlan};

fn main() {
    let files = std::env::var("WEAVATRIX_INCREMENTAL_N")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(1_000);
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("scan-incremental-{nonce}"));
    fs::create_dir_all(root.join("src")).unwrap();
    for index in 0..files {
        fs::write(root.join(format!("src/f{index}.rs")), "fn f() {}\n").unwrap();
    }
    let options = ScanOptions::default()
        .with_extensions(["rs"])
        .metadata_only()
        .selected_files_only();
    let previous = Scanner::new(&root).options(options.clone()).scan().unwrap();
    let started = Instant::now();
    let updated = Scanner::new(&root)
        .options(options)
        .scan_watch_plan(
            &previous,
            &WatchPlan {
                changed: vec!["src/f0.rs".to_owned()],
                removed: Vec::new(),
                full_rescan: false,
                rejected_events: 0,
            },
        )
        .unwrap();
    println!(
        "n={} k=1 update_ms={:.3} files={}",
        previous.files.len(),
        started.elapsed().as_secs_f64() * 1000.0,
        updated.files.len()
    );
    black_box(updated);
    let _ = fs::remove_dir_all(root);
}
