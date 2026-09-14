#![deny(unsafe_op_in_unsafe_fn)]

use decode::{SnapshotKind, decode_options, encode_report, scan_error};
use napi::bindgen_prelude::AsyncTask;
use napi::{Env, Result, Task};
use napi_derive::napi;
use session::OpenSessionTask;
use std::path::PathBuf;
use weavatrix_scan::{CancellationToken, ScanOptions, Scanner};

mod decode;
mod session;

const PACKAGE_VERSION: &str = env!("CARGO_PKG_VERSION");

#[napi(js_name = "CancellationToken")]
pub struct JsCancellationToken {
    pub(crate) inner: CancellationToken,
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
    kind: SnapshotKind,
}

impl Task for ScanTask {
    type Output = String;
    type JsValue = String;

    fn compute(&mut self) -> Result<Self::Output> {
        snapshot(&self.root, self.options.clone(), self.kind)
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
    let (options, kind) = decode_options(options_json, cancellation)?;
    Ok(AsyncTask::new(ScanTask {
        root: PathBuf::from(root),
        options,
        kind,
    }))
}

#[napi]
pub fn scan_repository_sync(
    root: String,
    options_json: Option<String>,
    cancellation: Option<&JsCancellationToken>,
) -> Result<String> {
    let (options, kind) = decode_options(options_json, cancellation)?;
    snapshot(&PathBuf::from(root), options, kind)
}

pub struct ScanPathsTask {
    root: PathBuf,
    options: ScanOptions,
}

impl Task for ScanPathsTask {
    type Output = Vec<String>;
    type JsValue = Vec<String>;

    fn compute(&mut self) -> Result<Self::Output> {
        collect_paths(&self.root, self.options.clone())
    }

    fn resolve(&mut self, _env: Env, output: Self::Output) -> Result<Self::JsValue> {
        Ok(output)
    }
}

#[napi]
pub fn scan_paths(
    root: String,
    options_json: Option<String>,
    cancellation: Option<&JsCancellationToken>,
) -> Result<AsyncTask<ScanPathsTask>> {
    let (options, _) = decode_options(options_json, cancellation)?;
    Ok(AsyncTask::new(ScanPathsTask {
        root: PathBuf::from(root),
        options,
    }))
}

#[napi]
pub fn scan_paths_sync(
    root: String,
    options_json: Option<String>,
    cancellation: Option<&JsCancellationToken>,
) -> Result<Vec<String>> {
    let (options, _) = decode_options(options_json, cancellation)?;
    collect_paths(&PathBuf::from(root), options)
}

#[napi]
pub fn export_scan_cache(
    root: String,
    options_json: Option<String>,
    cancellation: Option<&JsCancellationToken>,
) -> Result<AsyncTask<ScanTask>> {
    let (options, _) = decode_options(options_json, cancellation)?;
    Ok(AsyncTask::new(ScanTask {
        root: PathBuf::from(root),
        options,
        kind: SnapshotKind::Cache,
    }))
}

#[napi]
pub fn export_scan_cache_sync(
    root: String,
    options_json: Option<String>,
    cancellation: Option<&JsCancellationToken>,
) -> Result<String> {
    let (options, _) = decode_options(options_json, cancellation)?;
    snapshot(&PathBuf::from(root), options, SnapshotKind::Cache)
}

#[napi]
pub fn open_scan_session(
    root: String,
    options_json: Option<String>,
    cancellation: Option<&JsCancellationToken>,
) -> Result<AsyncTask<OpenSessionTask>> {
    let (options, _) = decode_options(options_json, cancellation)?;
    Ok(AsyncTask::new(OpenSessionTask {
        root: PathBuf::from(root),
        options,
    }))
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

fn snapshot(root: &PathBuf, options: ScanOptions, kind: SnapshotKind) -> Result<String> {
    let report = Scanner::new(root)
        .options(options)
        .scan()
        .map_err(scan_error)?;
    encode_report(&report, kind)
}

fn collect_paths(root: &PathBuf, options: ScanOptions) -> Result<Vec<String>> {
    Scanner::new(root)
        .options(options)
        .scan_paths()
        .map_err(scan_error)
}
