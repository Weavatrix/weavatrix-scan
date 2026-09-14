use crate::JsCancellationToken;
use crate::decode::{
    SnapshotKind, decode_options, decode_plan, encode_report, json_error, scan_error,
};
use napi::bindgen_prelude::AsyncTask;
use napi::{Env, Error, Result, Status, Task};
use napi_derive::napi;
use serde::Serialize;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use weavatrix_scan::{CancellationToken, ScanOptions, ScanSession};

pub struct OpenSessionTask {
    pub root: PathBuf,
    pub options: ScanOptions,
}

impl Task for OpenSessionTask {
    type Output = ScanSession;
    type JsValue = JsScanSession;

    fn compute(&mut self) -> Result<Self::Output> {
        ScanSession::open(self.root.clone(), self.options.clone()).map_err(scan_error)
    }

    fn resolve(&mut self, _env: Env, output: Self::Output) -> Result<Self::JsValue> {
        Ok(JsScanSession::from_session(output))
    }
}

pub struct ApplyWatchTask {
    inner: Arc<Mutex<ScanSession>>,
    plan_json: String,
    kind: SnapshotKind,
    cancellation: Option<CancellationToken>,
}

impl Task for ApplyWatchTask {
    type Output = String;
    type JsValue = String;

    fn compute(&mut self) -> Result<Self::Output> {
        let plan = decode_plan(&self.plan_json)?;
        let mut session = self.inner.lock().map_err(lock_error)?;
        session
            .apply_watch_plan_with_cancellation(&plan, self.cancellation.clone())
            .map_err(scan_error)?;
        encode_report(session.report(), self.kind)
    }

    fn resolve(&mut self, _env: Env, output: Self::Output) -> Result<Self::JsValue> {
        Ok(output)
    }
}

#[derive(Serialize)]
struct FilesPage {
    generation: u64,
    files: Vec<weavatrix_scan::PortableScannedFile>,
}

#[derive(Serialize)]
struct FileCursor {
    generation: u64,
    count: u32,
}

#[napi(js_name = "ScanSession")]
pub struct JsScanSession {
    inner: Arc<Mutex<ScanSession>>,
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
        Ok(Self::from_session(
            ScanSession::open(PathBuf::from(root), options).map_err(scan_error)?,
        ))
    }

    #[napi]
    pub fn snapshot(&self, compact: Option<bool>) -> Result<String> {
        let session = self.inner.lock().map_err(lock_error)?;
        encode_report(session.report(), snapshot_kind(compact))
    }

    #[napi]
    pub fn export_cache(&self) -> Result<String> {
        let session = self.inner.lock().map_err(lock_error)?;
        encode_report(session.report(), SnapshotKind::Cache)
    }

    #[napi]
    pub fn update_reason(&self) -> Result<Option<String>> {
        let session = self.inner.lock().map_err(lock_error)?;
        Ok(session
            .last_update_reason()
            .map(|reason| reason.as_str().to_owned()))
    }

    #[napi]
    pub fn apply_watch_plan(
        &self,
        plan_json: String,
        compact: Option<bool>,
        cancellation: Option<&JsCancellationToken>,
    ) -> Result<String> {
        let plan = decode_plan(&plan_json)?;
        let mut session = self.inner.lock().map_err(lock_error)?;
        session
            .apply_watch_plan_with_cancellation(
                &plan,
                cancellation.map(|token| token.inner.clone()),
            )
            .map_err(scan_error)?;
        encode_report(session.report(), snapshot_kind(compact))
    }

    #[napi]
    pub fn apply_watch_plan_async(
        &self,
        plan_json: String,
        compact: Option<bool>,
        cancellation: Option<&JsCancellationToken>,
    ) -> Result<AsyncTask<ApplyWatchTask>> {
        Ok(AsyncTask::new(ApplyWatchTask {
            inner: Arc::clone(&self.inner),
            plan_json,
            kind: snapshot_kind(compact),
            cancellation: cancellation.map(|token| token.inner.clone()),
        }))
    }

    #[napi]
    pub fn files_page(&self, generation: u32, offset: u32, limit: u32) -> Result<String> {
        let session = self.inner.lock().map_err(lock_error)?;
        let files = session
            .files_page(
                u64::from(generation),
                usize::try_from(offset).unwrap_or(usize::MAX),
                usize::try_from(limit).unwrap_or(0),
            )
            .map_err(scan_error)?;
        serde_json::to_string(&FilesPage {
            generation: session.generation(),
            files,
        })
        .map_err(json_error)
    }

    #[napi]
    pub fn file_cursor(&self) -> Result<String> {
        let session = self.inner.lock().map_err(lock_error)?;
        let count = u32::try_from(session.report().files.len()).map_err(scan_error)?;
        serde_json::to_string(&FileCursor {
            generation: session.generation(),
            count,
        })
        .map_err(json_error)
    }

    #[napi]
    pub fn file_count(&self) -> Result<u32> {
        let session = self.inner.lock().map_err(lock_error)?;
        u32::try_from(session.report().files.len()).map_err(scan_error)
    }
}

impl JsScanSession {
    pub(crate) fn from_session(session: ScanSession) -> Self {
        Self {
            inner: Arc::new(Mutex::new(session)),
        }
    }
}

fn snapshot_kind(compact: Option<bool>) -> SnapshotKind {
    if compact.unwrap_or(false) {
        SnapshotKind::Compact
    } else {
        SnapshotKind::Portable
    }
}

fn lock_error(_: std::sync::PoisonError<std::sync::MutexGuard<'_, ScanSession>>) -> Error {
    Error::new(Status::GenericFailure, "scan session lock was poisoned")
}
