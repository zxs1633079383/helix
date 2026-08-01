//! Persist / Http BoundedSpawner 构造助手（泛型 over Storage / HttpRequester）。
//!
//! - Persist：per-table-set 懒建，每实例 N=1（保写序），Block 溢出。
//! - Http(必达)：全局单例 N=max_http_inflight，Block 溢出。
//! - HttpFire(可丢)：全局单例 N，DropNewest 溢出（满即丢当前条目）。

use std::collections::HashMap;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;
use std::time::Instant;

use bytes::Bytes;
use serde_json::json;
use tokio::sync::mpsc;

use helix_core::effect::{
    FileUploadProgress, FileUploadProgressPolicy, FileUploadRequest, HttpRequest, StorageOp,
};
use helix_core::ports::{FileUploadProgressReporter, FileUploader, HttpRequester, Storage};
use helix_core::tick::{PortOutcome, ReplyBytes};
use helix_core::{Correlation, Tick};

use crate::http_envelope::run_http_envelope;
use crate::ops::classify_port_error;
use crate::spawner::{BoundedSpawner, Overflow};
use crate::storage::{with_storage_trace_context, StorageTraceContext};
use crate::{execute_storage_ops, execute_storage_ops_atomic, PERSIST_QUEUE_CAP};

use crate::HTTP_QUEUE_CAP;

/// 只保留 HTTP URL 的 scheme/host/path，禁止 query 或 fragment 进入运行日志。
fn safe_http_target(url: &str) -> &str {
    url.split(['?', '#']).next().unwrap_or(url)
}

/// 从共享 HTTP 信封读取状态码；解析失败保持 None，绝不记录响应正文。
fn http_status(outcome: &PortOutcome) -> Option<u64> {
    let PortOutcome::Ok(reply) = outcome else {
        return None;
    };
    serde_json::from_slice::<serde_json::Value>(&reply.0)
        .ok()
        .and_then(|value| value.get("status").and_then(serde_json::Value::as_u64))
}

pub struct PersistJobPayload {
    pub ops: Vec<StorageOp>,
    /// `Effect::PersistAtomic` opt-in; ordinary Persist/PersistFire retain their existing semantics.
    pub atomic: bool,
    pub trace: Option<StorageTraceContext>,
}

struct UploadProgressTickReporter {
    corr: Correlation,
    percent_step: u8,
    last_emitted_percent: AtomicU8,
    reply_tx: Arc<mpsc::UnboundedSender<Tick>>,
}

impl UploadProgressTickReporter {
    fn new(
        corr: Correlation,
        percent_step: u8,
        reply_tx: Arc<mpsc::UnboundedSender<Tick>>,
    ) -> Option<Self> {
        (1..=100).contains(&percent_step).then_some(Self {
            corr,
            percent_step,
            last_emitted_percent: AtomicU8::new(0),
            reply_tx,
        })
    }
}

impl FileUploadProgressReporter for UploadProgressTickReporter {
    fn report(&self, progress: FileUploadProgress) {
        if progress.total_bytes == 0 || progress.completed_bytes > progress.total_bytes {
            return;
        }
        let raw_percent =
            ((u128::from(progress.completed_bytes) * 100) / u128::from(progress.total_bytes)) as u8;
        let bucket = if raw_percent == 100 {
            100
        } else {
            (raw_percent / self.percent_step) * self.percent_step
        };
        if bucket == 0 {
            return;
        }

        let mut seen = self.last_emitted_percent.load(Ordering::Relaxed);
        while bucket > seen {
            match self.last_emitted_percent.compare_exchange_weak(
                seen,
                bucket,
                Ordering::AcqRel,
                Ordering::Relaxed,
            ) {
                Ok(_) => {
                    self.reply_tx
                        .send(Tick::PortProgress {
                            corr: self.corr,
                            progress,
                        })
                        .ok();
                    return;
                }
                Err(actual) => seen = actual,
            }
        }
    }
}

/// 启动（或复用）某 table-set 对应的 Persist BoundedSpawner（N=1 单消费者保序）。
///
/// ## 设计不变量（不变量3）
///
/// 同 key 内所有 Persist/PersistFire 严格按入队顺序执行（N=1 → 单 worker 单消费者）。
/// job queue 有界（`PERSIST_QUEUE_CAP`），超限时 caller `.await` 阻塞，背压传导到 tick_tx。
pub fn get_or_spawn_persist<S>(
    workers: &mut HashMap<String, BoundedSpawner<PersistJobPayload>>,
    table_key: String,
    storage: Arc<S>,
    reply_tx: mpsc::UnboundedSender<Tick>,
) -> &BoundedSpawner<PersistJobPayload>
where
    S: Storage + Send + Sync + 'static,
{
    workers.entry(table_key).or_insert_with(|| {
        let storage = Arc::clone(&storage);
        BoundedSpawner::new(
            1, // N=1：单 worker 单消费者保写序（不变量3）
            PERSIST_QUEUE_CAP,
            Overflow::Block,
            reply_tx,
            move |_corr, payload: PersistJobPayload| {
                let storage = Arc::clone(&storage);
                async move {
                    let PersistJobPayload { ops, atomic, trace } = payload;
                    let execute = async {
                        if atomic {
                            execute_storage_ops_atomic(storage.as_ref(), &ops).await
                        } else {
                            execute_storage_ops(storage.as_ref(), &ops).await
                        }
                    };
                    match trace {
                        Some(trace) => with_storage_trace_context(trace, execute).await,
                        None => execute.await,
                    }
                }
            },
        )
    })
}

/// 构造 Http BoundedSpawner（必达 Block / 可丢 DropNewest 由 overflow 参数选）。
///
/// N=concurrency 个 worker 抢有界队列，泵只入队不发请求——第 N+1 条在队列里等 worker。
pub fn spawn_http_pool<H>(
    concurrency: usize,
    http: Arc<H>,
    reply_tx: mpsc::UnboundedSender<Tick>,
    overflow: Overflow,
) -> BoundedSpawner<HttpRequest>
where
    H: HttpRequester + Send + Sync + 'static,
{
    BoundedSpawner::new(
        concurrency,
        HTTP_QUEUE_CAP,
        overflow,
        reply_tx,
        move |corr, req: HttpRequest| {
            let http = Arc::clone(&http);
            async move {
                let started_at = Instant::now();
                let method = req.method.clone();
                let target = safe_http_target(&req.url).to_string();
                tracing::info!(
                    hop = "host.http.dispatch",
                    corr = corr.map(Correlation::raw),
                    method,
                    target,
                    "HTTP effect entered the host worker"
                );
                let outcome = run_http_envelope(http.as_ref(), req).await;
                let outcome_class = if matches!(outcome, PortOutcome::Ok(_)) {
                    "ok"
                } else {
                    "err"
                };
                tracing::info!(
                    hop = "host.http.complete",
                    corr = corr.map(Correlation::raw),
                    method,
                    target,
                    status = http_status(&outcome),
                    outcome = outcome_class,
                    elapsed_ms = started_at.elapsed().as_millis() as u64,
                    "HTTP effect completed without exposing headers or body"
                );
                outcome
            }
        },
    )
}

/// 构造 Upload BoundedSpawner（必达 Block）。
pub fn spawn_upload_pool<U>(
    concurrency: usize,
    uploader: Arc<U>,
    reply_tx: mpsc::UnboundedSender<Tick>,
    overflow: Overflow,
) -> BoundedSpawner<FileUploadRequest>
where
    U: FileUploader + Send + Sync + 'static,
{
    let progress_reply_tx = Arc::new(reply_tx.clone());
    BoundedSpawner::new(
        concurrency,
        HTTP_QUEUE_CAP,
        overflow,
        reply_tx,
        move |corr, req: FileUploadRequest| {
            let uploader = Arc::clone(&uploader);
            let progress_reply_tx = Arc::clone(&progress_reply_tx);
            async move {
                let correlation = corr.map(Correlation::raw).unwrap_or_default();
                tracing::info!(
                    hop = "host.upload.dispatch",
                    corr = correlation,
                    object_key = req.object_key.as_str(),
                    size = req.size.unwrap_or_default(),
                    "OSS upload dispatched without exposing its signed URL"
                );
                let progress = match (corr, req.progress_policy()) {
                    (Some(corr), FileUploadProgressPolicy::PercentStep(step)) => {
                        UploadProgressTickReporter::new(corr, step, progress_reply_tx).map(
                            |reporter| Arc::new(reporter) as Arc<dyn FileUploadProgressReporter>,
                        )
                    }
                    _ => None,
                };
                match uploader.upload_with_progress(req, progress).await {
                    Ok(resp) => {
                        tracing::info!(
                            hop = "host.upload.complete",
                            corr = correlation,
                            outcome = "ok",
                            object_key = resp.object_key.as_str(),
                            "OSS upload completed"
                        );
                        let bytes = serde_json::to_vec(&json!({
                            "objectKey": resp.object_key,
                            // PortReply wire 保留兼容键 `url`，值只来自长期可读地址。
                            "url": resp.public_url,
                            "etag": resp.etag,
                        }))
                        .expect("upload response JSON serialization cannot fail");
                        PortOutcome::Ok(ReplyBytes(Bytes::from(bytes)))
                    }
                    Err(e) => {
                        tracing::warn!(
                            hop = "host.upload.complete",
                            corr = correlation,
                            outcome = "err",
                            "OSS upload failed without exposing its signed URL"
                        );
                        PortOutcome::Err(classify_port_error(e))
                    }
                }
            }
        },
    )
}
