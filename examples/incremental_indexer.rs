//! A small Rust indexer that commits files only after a successful
//! [`ContentVisitEvent::FileEnd`].

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use weavatrix_scan::{
    ContentFileStatus, ContentVisitControl, ContentVisitEvent, ScanOptions, ScanSession, Scanner,
};

fn main() -> weavatrix_scan::Result<()> {
    let options = ScanOptions::default()
        .with_extensions(["rs"])
        .selected_files_only();
    let session = ScanSession::open(".", options.clone())?;
    println!(
        "manifest {} files revision {}",
        session.report().files.len(),
        session.report().revision
    );

    let committed = Arc::new(AtomicU64::new(0));
    Scanner::new(".")
        .options(options)
        .visit_content_streaming({
            let committed = Arc::clone(&committed);
            move |_| {
                let committed = Arc::clone(&committed);
                let mut pending = Vec::new();
                move |event| match event {
                    ContentVisitEvent::FileStart { .. } => {
                        pending.clear();
                        ContentVisitControl::Continue
                    }
                    ContentVisitEvent::Chunk { bytes, .. } => {
                        pending.extend_from_slice(bytes);
                        ContentVisitControl::Continue
                    }
                    ContentVisitEvent::FileEnd { status, .. } => {
                        if status == ContentFileStatus::Selected {
                            committed.fetch_add(1, Ordering::Relaxed);
                        }
                        pending.clear();
                        ContentVisitControl::Continue
                    }
                }
            }
        })?;
    println!(
        "committed {} files after FileEnd",
        committed.load(Ordering::Relaxed)
    );
    Ok(())
}
