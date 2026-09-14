use std::fs;
use std::hint::black_box;
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use weavatrix_scan::{ScanOptions, Scanner, WatchPlan, WatchUpdateReason};

fn main() {
    let files = std::env::var("WEAVATRIX_INCREMENTAL_N")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(1_000);
    let prefixes = std::env::var("WEAVATRIX_INCREMENTAL_K")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(64_usize);
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("scan-incremental-{nonce}"));
    fs::create_dir_all(root.join("src/nested")).unwrap();
    fs::create_dir_all(root.join("right")).unwrap();
    fs::write(root.join("right/.gitignore"), "# nested\n").unwrap();
    fs::write(root.join("right/keep.rs"), "fn keep() {}\n").unwrap();
    for index in 0..files {
        fs::write(root.join(format!("src/f{index}.rs")), "fn f() {}\n").unwrap();
    }
    let options = ScanOptions::default()
        .with_extensions(["rs"])
        .metadata_only()
        .selected_files_only();
    let previous = Scanner::new(&root).options(options.clone()).scan().unwrap();

    let single_plan = WatchPlan {
        changed: vec!["src/f0.rs".to_owned()],
        removed: Vec::new(),
        full_rescan: false,
        rejected_events: 0,
    };
    let single = measure(&root, &options, &previous, &single_plan);
    let overlapping = (0..prefixes.min(files))
        .map(|index| format!("src/f{index}.rs"))
        .collect::<Vec<_>>();
    let batch_plan = WatchPlan {
        changed: overlapping,
        removed: Vec::new(),
        full_rescan: false,
        rejected_events: 0,
    };
    let batch = measure(&root, &options, &previous, &batch_plan);
    let nested_plan = WatchPlan {
        changed: vec!["src/f1.rs".to_owned()],
        removed: Vec::new(),
        full_rescan: false,
        rejected_events: 0,
    };
    let nested = measure(&root, &options, &previous, &nested_plan);
    println!(
        "n={} k1_ms={:.3} k1_reason={} k{}_ms={:.3} k{}_reason={} nested_ignore_ms={:.3} nested_reason={}",
        previous.files.len(),
        single.0,
        single.1,
        prefixes,
        batch.0,
        prefixes,
        batch.1,
        nested.0,
        nested.1
    );
    black_box((single, batch, nested));
    let _ = fs::remove_dir_all(root);
}

fn measure(
    root: &std::path::Path,
    options: &ScanOptions,
    previous: &weavatrix_scan::ScanReport,
    plan: &WatchPlan,
) -> (f64, WatchUpdateReason) {
    let started = Instant::now();
    let update = Scanner::new(root)
        .options(options.clone())
        .scan_watch_plan_detailed(previous, plan)
        .unwrap();
    (started.elapsed().as_secs_f64() * 1000.0, update.reason)
}
