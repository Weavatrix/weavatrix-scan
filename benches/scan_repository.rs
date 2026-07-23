mod support;

use support::{EXTENSIONS, Fixture, IGNORE_AWARE_FILES, SOURCE_FILES, measure, print_measurement};
use weavatrix_scan::{ScanOptions, Scanner};

fn main() {
    let fixture = Fixture::new();
    println!("corpus=synthetic source_files={SOURCE_FILES} statistic=median runs=11 warmups=2");

    let rich = measure(|| {
        Scanner::new(&fixture.root)
            .options(ScanOptions::default().with_extensions(EXTENSIONS))
            .scan()
            .unwrap()
            .files
            .len()
    });
    let safe = measure(|| {
        let mut options = ScanOptions::default().with_extensions(EXTENSIONS);
        options.hash_file_contents = false;
        Scanner::new(&fixture.root)
            .options(options)
            .scan()
            .unwrap()
            .files
            .len()
    });
    let metadata = measure(|| {
        Scanner::new(&fixture.root)
            .options(
                ScanOptions::default()
                    .with_extensions(EXTENSIONS)
                    .metadata_only(),
            )
            .scan()
            .unwrap()
            .files
            .len()
    });

    assert_eq!(rich.count, SOURCE_FILES);
    assert_eq!(safe.count, SOURCE_FILES);
    assert_eq!(metadata.count, IGNORE_AWARE_FILES);
    print_measurement("rich-manifest", "weavatrix-scan", &rich);
    print_measurement("safe-discovery", "weavatrix-scan", &safe);
    print_measurement("metadata-only", "weavatrix-scan", &metadata);
}
