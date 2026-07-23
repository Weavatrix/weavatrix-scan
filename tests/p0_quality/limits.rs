use super::support::Fixture;
use std::time::Duration;
use weavatrix_scan::{CancellationToken, ScanOptions, ScanTermination, Scanner};

#[test]
fn scan_limits_and_cancellation_return_typed_partial_reports() {
    let fixture = Fixture::new("weavatrix-p0-limits");
    fixture.write("large.rs", "0123456789");

    let entries = Scanner::new(&fixture.root)
        .options(ScanOptions::default().with_max_entries(Some(1)))
        .scan()
        .unwrap();
    assert_eq!(entries.termination, Some(ScanTermination::MaxEntries));
    assert!(!entries.complete);

    let bytes = Scanner::new(&fixture.root)
        .options(
            ScanOptions::default()
                .with_extensions(["rs"])
                .with_max_total_bytes(Some(4)),
        )
        .scan()
        .unwrap();
    assert_eq!(bytes.termination, Some(ScanTermination::MaxTotalBytes));
    assert!(bytes.files.is_empty());

    let timeout = Scanner::new(&fixture.root)
        .options(ScanOptions::default().with_timeout(Some(Duration::ZERO)))
        .scan()
        .unwrap();
    assert_eq!(timeout.termination, Some(ScanTermination::Timeout));

    let token = CancellationToken::new();
    token.cancel();
    let cancelled = Scanner::new(&fixture.root)
        .options(ScanOptions::default().with_cancellation(token))
        .scan()
        .unwrap();
    assert_eq!(cancelled.termination, Some(ScanTermination::Cancelled));

    let fits = Scanner::new(&fixture.root)
        .options(
            ScanOptions::default()
                .with_extensions(["rs"])
                .with_max_total_bytes(Some(10)),
        )
        .scan()
        .unwrap();
    assert_eq!(fits.termination, None);
    assert_eq!(fits.files.len(), 1);

    let selected_only = Scanner::new(&fixture.root)
        .options(
            ScanOptions::default()
                .with_extensions(["rs"])
                .with_max_total_bytes(Some(1))
                .selected_files_only(),
        )
        .scan()
        .unwrap();
    assert_eq!(
        selected_only.termination,
        Some(ScanTermination::MaxTotalBytes)
    );
    assert!(selected_only.skipped.is_empty());
}
