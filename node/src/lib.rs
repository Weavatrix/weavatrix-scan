#![deny(unsafe_op_in_unsafe_fn)]

use napi::bindgen_prelude::AsyncTask;
use napi::{Env, Error, Result, Status, Task};
use napi_derive::napi;
use serde::Deserialize;
use std::path::PathBuf;
use weavatrix_scan::{ScanOptions, Scanner};

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
struct NodeScanOptions {
    extensions: Vec<String>,
    override_rules: Vec<String>,
    metadata_only: bool,
    selected_files_only: bool,
    skip_hidden: Option<bool>,
    max_file_bytes: Option<u64>,
    max_entries: Option<u64>,
    max_total_bytes: Option<u64>,
    max_depth: Option<usize>,
    parallelism: Option<usize>,
}

pub struct ScanTask {
    root: PathBuf,
    options: ScanOptions,
}

impl Task for ScanTask {
    type Output = String;
    type JsValue = String;

    fn compute(&mut self) -> Result<Self::Output> {
        scan(&self.root, self.options.clone())
    }

    fn resolve(&mut self, _env: Env, output: Self::Output) -> Result<Self::JsValue> {
        Ok(output)
    }
}

#[napi]
pub fn scan_repository(root: String, options_json: Option<String>) -> Result<AsyncTask<ScanTask>> {
    Ok(AsyncTask::new(ScanTask {
        root: PathBuf::from(root),
        options: decode_options(options_json)?,
    }))
}

#[napi]
pub fn scan_repository_sync(root: String, options_json: Option<String>) -> Result<String> {
    scan(&PathBuf::from(root), decode_options(options_json)?)
}

fn scan(root: &PathBuf, options: ScanOptions) -> Result<String> {
    let report = Scanner::new(root)
        .options(options)
        .scan()
        .map_err(scan_error)?;
    serde_json::to_string(&report.to_portable()).map_err(json_error)
}

fn decode_options(options_json: Option<String>) -> Result<ScanOptions> {
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
    if input.selected_files_only {
        options = options.selected_files_only();
    }
    if let Some(value) = input.skip_hidden {
        options = options.with_skip_hidden(value);
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
    Ok(options)
}

fn json_error(error: serde_json::Error) -> Error {
    Error::new(Status::InvalidArg, error.to_string())
}

fn scan_error(error: impl core::fmt::Display) -> Error {
    Error::new(Status::GenericFailure, error.to_string())
}
