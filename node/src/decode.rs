use crate::JsCancellationToken;
use napi::{Error, Result, Status};
use serde::Deserialize;
use weavatrix_scan::{IgnorePolicy, ScanOptions, StandardSkips, WatchPlan};

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct NodeScanOptions {
    extensions: Vec<String>,
    override_rules: Vec<String>,
    metadata_only: bool,
    selected_files_only: bool,
    skip_hidden: Option<bool>,
    standard_skips: Option<bool>,
    ignore_policy: Option<String>,
    hash_file_contents: Option<bool>,
    max_file_bytes: Option<u64>,
    max_entries: Option<u64>,
    max_total_bytes: Option<u64>,
    max_depth: Option<usize>,
    parallelism: Option<usize>,
    compact: bool,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
struct NodeWatchPlan {
    changed: Vec<String>,
    removed: Vec<String>,
    full_rescan: bool,
    rejected_events: u64,
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum SnapshotKind {
    Portable,
    Compact,
    Cache,
}

pub(crate) fn decode_options(
    options_json: Option<String>,
    cancellation: Option<&JsCancellationToken>,
) -> Result<(ScanOptions, SnapshotKind)> {
    let raw = options_json.unwrap_or_else(|| "{}".to_owned());
    let input: NodeScanOptions = serde_json::from_str(&raw).map_err(json_error)?;
    let mut options = ScanOptions::default();
    if !input.extensions.is_empty() {
        options = options.with_extensions(input.extensions);
    }
    if !input.override_rules.is_empty() {
        options = options.with_override_rules(input.override_rules);
    }
    if input.metadata_only {
        options = options.metadata_only();
    }
    if let Some(false) = input.hash_file_contents {
        options = options.metadata_only();
    }
    if input.selected_files_only {
        options = options.selected_files_only();
    }
    if let Some(value) = input.skip_hidden {
        options = options.with_skip_hidden(value);
    }
    if let Some(enabled) = input.standard_skips {
        options = options.with_standard_skips(if enabled {
            StandardSkips::Enabled
        } else {
            StandardSkips::Disabled
        });
    }
    if let Some(policy) = input.ignore_policy.as_deref() {
        options = options.with_ignore_policy(match policy {
            "none" => IgnorePolicy::none(),
            "gitCompatible" | "git-compatible" => IgnorePolicy::git_compatible(),
            "repository" => IgnorePolicy::repository(),
            other => {
                return Err(Error::new(
                    Status::InvalidArg,
                    format!("unsupported ignorePolicy: {other}"),
                ));
            }
        });
    }
    if let Some(value) = input.max_file_bytes {
        options.max_file_bytes = value;
    }
    if input.max_entries.is_some() {
        options = options.with_max_entries(input.max_entries);
    }
    if input.max_total_bytes.is_some() {
        options = options.with_max_total_bytes(input.max_total_bytes);
    }
    if input.max_depth.is_some() {
        options = options.with_max_depth(input.max_depth);
    }
    if let Some(value) = input.parallelism {
        options = options.with_parallelism(value);
    }
    if let Some(token) = cancellation {
        options = options.with_cancellation(token.inner.clone());
    }
    Ok((
        options,
        if input.compact {
            SnapshotKind::Compact
        } else {
            SnapshotKind::Portable
        },
    ))
}

pub(crate) fn decode_plan(plan_json: &str) -> Result<WatchPlan> {
    let input: NodeWatchPlan = serde_json::from_str(plan_json).map_err(json_error)?;
    Ok(WatchPlan {
        changed: input.changed,
        removed: input.removed,
        full_rescan: input.full_rescan,
        rejected_events: input.rejected_events,
    })
}

pub(crate) fn encode_report(
    report: &weavatrix_scan::ScanReport,
    kind: SnapshotKind,
) -> Result<String> {
    match kind {
        SnapshotKind::Cache => serde_json::to_string(&report.to_cache()).map_err(json_error),
        SnapshotKind::Compact => serde_json::to_string(&report.to_compact()).map_err(json_error),
        SnapshotKind::Portable => serde_json::to_string(&report.to_portable()).map_err(json_error),
    }
}

pub(crate) fn json_error(error: serde_json::Error) -> Error {
    Error::new(Status::InvalidArg, error.to_string())
}

pub(crate) fn scan_error(error: impl core::fmt::Display) -> Error {
    Error::new(Status::GenericFailure, error.to_string())
}
