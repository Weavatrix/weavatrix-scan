use crate::support::{EXTENSIONS, Measurement};
use std::path::Path;

pub(crate) fn has_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|value| value.to_str())
        .is_some_and(|value| EXTENSIONS.contains(&value.to_ascii_lowercase().as_str()))
}

pub(crate) fn print_count(mode: &str, library: &str, unit: &str, measurement: &Measurement) {
    println!(
        "mode={mode} library={library} {unit}={} median_ms={:.3} min_ms={:.3}",
        measurement.count,
        measurement.median.as_secs_f64() * 1_000.0,
        measurement.minimum.as_secs_f64() * 1_000.0
    );
}
