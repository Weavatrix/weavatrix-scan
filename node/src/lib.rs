#![deny(unsafe_op_in_unsafe_fn)]

use napi::bindgen_prelude::AsyncTask;
use napi::{Env, Error, Result, Status, Task};
use napi_derive::napi;
use serde::Deserialize;
use std::path::PathBuf;
use std::sync::Mutex;
use weavatrix_scan::{
    CancellationToken, IgnorePolicy, ScanOptions, ScanSession, Scanner, StandardSkips, WatchPlan,
};

const PACKAGE_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
struct NodeScanOptions {
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

#[napi(js_name = "CancellationToken")]
pub struct JsCancellationToken {
    inner: CancellationToken,
}

#[napi]
impl JsCancellationToken {
    #[napi(constructor)]
    pub fn new() -> Self {
        Self {
            inner: CancellationToken::new(),
        }
    }

    #[napi]
    pub fn cancel(&self) {
        self.inner.cancel();
    }

    #[napi]
    pub fn is_cancelled(&self) -> bool {
        self.inner.is_cancelled()
    }
}

pub struct ScanTask {
    root: PathBuf,
    options: ScanOptions,
    compact: bool,
}

impl Task for ScanTask {
    type Output = String;
    type JsValue = String;

    fn compute(&mut self) -> Result<Self::Output> {
        snapshot(&self.root, self.options.clone(), self.compact)
    }

    fn resolve(&mut self, _env: Env, output: Self::Output) -> Result<Self::JsValue> {
        Ok(output)
    }
}

#[napi]
pub fn scan_repository(
    root: String,
    options_json: Option<String>,
    cancellation: Option<&JsCancellationToken>,
) -> Result<AsyncTask<ScanTask>> {
    let (options, compact) = decode_options(options_json, cancellation)?;
    Ok(AsyncTask::new(ScanTask {
        root: PathBuf::from(root),
        options,
        compact,
    }))
}

#[napi]
pub fn scan_repository_sync(
    root: String,
    options_json: Option<String>,
    cancellation: Option<&JsCancellationToken>,
) -> Result<String> {
    let (options, compact) = decode_options(options_json, cancellation)?;
    snapshot(&PathBuf::from(root), options, compact)
}

#[napi]
pub fn scan_diagnostics() -> String {
    let musl = cfg!(target_env = "musl");
    format!(
        "{{\"rustCoreVersion\":\"{}\",\"packageVersion\":\"{}\",\"target\":\"{}\",\"muslSupported\":false,\"muslBuild\":{musl},\"supportedTargets\":[\"x86_64-pc-windows-msvc\",\"aarch64-pc-windows-msvc\",\"x86_64-apple-darwin\",\"aarch64-apple-darwin\",\"x86_64-unknown-linux-gnu\",\"aarch64-unknown-linux-gnu\"]}}",
        env!("CARGO_PKG_VERSION"),
        PACKAGE_VERSION,
        format!("{}-{}", std::env::consts::ARCH, std::env::consts::OS)
    )
}

#[napi(js_name = "ScanSession")]
pub struct JsScanSession {
    inner: Mutex<ScanSession>,
}

#[napi]
impl JsScanSession {
    #[napi(factory)]
    pub fn open(
        root: String,
        options_json: Option<String>,
        cancellation: Option<&JsCancellationToken>,
    ) -> Result<Self> {
        let (options, _) = decode_options(options_json, cancellation)?;
        Ok(Self {
            inner: Mutex::new(ScanSession::open(PathBuf::from(root), options).map_err(scan_error)?),
        })
    }

    #[napi]
    pub fn snapshot(&self, compact: Option<bool>) -> Result<String> {
        let session = self.inner.lock().map_err(lock_error)?;
        encode_report(session.report(), compact.unwrap_or(false))
    }

    #[napi]
    pub fn apply_watch_plan(&self, plan_json: String, compact: Option<bool>) -> Result<String> {
        let plan = decode_plan(&plan_json)?;
        let mut session = self.inner.lock().map_err(lock_error)?;
        session.apply_watch_plan(&plan).map_err(scan_error)?;
        encode_report(session.report(), compact.unwrap_or(false))
    }

    #[napi]
    pub fn files_page(&self, offset: u32, limit: u32) -> Result<String> {
        let session = self.inner.lock().map_err(lock_error)?;
        let files = &session.report().to_portable().files;
        let start = usize::try_from(offset).unwrap_or(usize::MAX);
        let end = start.saturating_add(usize::try_from(limit).unwrap_or(0));
        let page = files.get(start..files.len().min(end)).unwrap_or(&[]);
        serde_json::to_string(page).map_err(json_error)
    }

    #[napi]
    pub fn file_count(&self) -> Result<u32> {
        let session = self.inner.lock().map_err(lock_error)?;
        u32::try_from(session.report().files.len()).map_err(scan_error)
    }
}

fn snapshot(root: &PathBuf, options: ScanOptions, compact: bool) -> Result<String> {
    let report = Scanner::new(root)
        .options(options)
        .scan()
        .map_err(scan_error)?;
    encode_report(&report, compact)
}

fn encode_report(report: &weavatrix_scan::ScanReport, compact: bool) -> Result<String> {
    if compact {
        serde_json::to_string(&report.to_cache()).map_err(json_error)
    } else {
        serde_json::to_string(&report.to_portable()).map_err(json_error)
    }
}

fn decode_plan(plan_json: &str) -> Result<WatchPlan> {
    let input: NodeWatchPlan = serde_json::from_str(plan_json).map_err(json_error)?;
    Ok(WatchPlan {
        changed: input.changed,
        removed: input.removed,
        full_rescan: input.full_rescan,
        rejected_events: input.rejected_events,
    })
}

fn decode_options(
    options_json: Option<String>,
    cancellation: Option<&JsCancellationToken>,
) -> Result<(ScanOptions, bool)> {
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
    Ok((options, input.compact))
}

fn json_error(error: serde_json::Error) -> Error {
    Error::new(Status::InvalidArg, error.to_string())
}

fn scan_error(error: impl core::fmt::Display) -> Error {
    Error::new(Status::GenericFailure, error.to_string())
}

fn lock_error(_: std::sync::PoisonError<std::sync::MutexGuard<'_, ScanSession>>) -> Error {
    Error::new(Status::GenericFailure, "scan session lock was poisoned")
}
