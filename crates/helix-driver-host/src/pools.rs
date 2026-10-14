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
use crate::metrics::{AsyncMetricSink, NoopMetricSink};
use crate::ops::classify_port_error;
use crate::spawner::{BoundedSpawner, FeedbackSender, FeedbackSink, Overflow};
use crate::storage::{
    with_storage_operation_context, with_storage_trace_context, StorageOperationContext,
    StorageTraceContext,
};
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
    reply_tx: FeedbackSink,
}

impl UploadProgressTickReporter {
    /// 构造有界百分比进度 reporter；非法步长不创建回灌源。
    fn new(corr: Correlation, percent_step: u8, reply_tx: FeedbackSink) -> Option<Self> {
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
                    self.reply_tx.send(Tick::PortProgress {
                        corr: self.corr,
                        progress,
                    });
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
pub(crate) fn get_or_spawn_persist<S>(
    workers: &mut HashMap<String, BoundedSpawner<PersistJobPayload>>,
    table_key: String,
    storage: Arc<S>,
    reply_tx: FeedbackSink,
    metrics: Arc<dyn AsyncMetricSink>,
) -> &BoundedSpawner<PersistJobPayload>
where
    S: Storage + Send + Sync + 'static,
{
    workers.entry(table_key).or_insert_with(|| {
        let storage = Arc::clone(&storage);
        BoundedSpawner::new_with_feedback(
            1, // N=1：单 worker 单消费者保写序（不变量3）
            PERSIST_QUEUE_CAP,
            Overflow::Block,
            reply_tx,
            metrics,
            "persist",
            move |corr, payload: PersistJobPayload| {
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
                    let operation_context = StorageOperationContext::new(corr);
                    with_storage_operation_context(operation_context, async move {
                        match trace {
                            Some(trace) => with_storage_trace_context(trace, execute).await,
                            None => execute.await,
                        }
                    })
                    .await
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
    spawn_http_pool_with_feedback(
        concurrency,
        http,
        FeedbackSink::Raw(reply_tx),
        overflow,
        Arc::new(NoopMetricSink),
    )
}

/// 构造带低基数 pool 指标的 HTTP 工作池。
pub(crate) fn spawn_http_pool_observed<H>(
    concurrency: usize,
    http: Arc<H>,
    reply_tx: FeedbackSender,
    overflow: Overflow,
    metrics: Arc<dyn AsyncMetricSink>,
) -> BoundedSpawner<HttpRequest>
where
    H: HttpRequester + Send + Sync + 'static,
{
    spawn_http_pool_with_feedback(
        concurrency,
        http,
        FeedbackSink::Stamped(reply_tx),
        overflow,
        metrics,
    )
}

/// 汇合测试 raw 回灌与生产 stamped 回灌的 HTTP pool 构造。
fn spawn_http_pool_with_feedback<H>(
    concurrency: usize,
    http: Arc<H>,
    feedback: FeedbackSink,
    overflow: Overflow,
    metrics: Arc<dyn AsyncMetricSink>,
) -> BoundedSpawner<HttpRequest>
where
    H: HttpRequester + Send + Sync + 'static,
{
    let pool = match overflow {
        Overflow::Block => "http",
        Overflow::DropNewest => "http_fire",
    };
    BoundedSpawner::new_with_feedback(
        concurrency,
        HTTP_QUEUE_CAP,
        overflow,
        feedback,
        metrics,
        pool,
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
    spawn_upload_pool_with_feedback(
        concurrency,
        uploader,
        FeedbackSink::Raw(reply_tx),
        overflow,
        Arc::new(NoopMetricSink),
    )
}

/// 构造带低基数 pool 指标的上传工作池。
pub(crate) fn spawn_upload_pool_observed<U>(
    concurrency: usize,
    uploader: Arc<U>,
    reply_tx: FeedbackSender,
    overflow: Overflow,
    metrics: Arc<dyn AsyncMetricSink>,
) -> BoundedSpawner<FileUploadRequest>
where
    U: FileUploader + Send + Sync + 'static,
{
    spawn_upload_pool_with_feedback(
        concurrency,
        uploader,
        FeedbackSink::Stamped(reply_tx.clone()),
        overflow,
        metrics,
    )
}

/// 汇合测试 raw 回灌与生产 stamped 回灌的 Upload pool 构造。
fn spawn_upload_pool_with_feedback<U>(
    concurrency: usize,
    uploader: Arc<U>,
    feedback: FeedbackSink,
    overflow: Overflow,
    metrics: Arc<dyn AsyncMetricSink>,
) -> BoundedSpawner<FileUploadRequest>
where
    U: FileUploader + Send + Sync + 'static,
{
    let progress_reply_tx = feedback.clone();
    BoundedSpawner::new_with_feedback(
        // Uploads have an independent ceiling even when the HTTP pool is much wider.
        concurrency.clamp(1, 6),
        HTTP_QUEUE_CAP,
        overflow,
        feedback,
        metrics,
        "upload",
        move |corr, req: FileUploadRequest| {
            let uploader = Arc::clone(&uploader);
            let progress_reply_tx = progress_reply_tx.clone();
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

#[cfg(test)]
mod upload_budget_tests {
    use super::*;
    use crate::metrics::{MetricEvent, MetricId, RecordOutcome};
    use helix_core::effect::FileUploadResponse;
    use helix_core::PortError;
    use std::sync::Mutex;

    #[derive(Default)]
    struct Facts(Mutex<Vec<MetricEvent>>);
    impl AsyncMetricSink for Facts {
        /// Capture the constructed pool's worker facts, without a live exporter.
        fn try_record(&self, event: MetricEvent) -> RecordOutcome {
            self.0.lock().unwrap().push(event);
            RecordOutcome::Accepted
        }
    }
    struct IdleUploader;
    #[async_trait::async_trait]
    impl FileUploader for IdleUploader {
        /// No jobs are submitted; only construction and shutdown are exercised.
        async fn upload(&self, _: FileUploadRequest) -> Result<FileUploadResponse, PortError> {
            Err(PortError::Other("unexpected job".into()))
        }
    }

    /// High HTTP concurrency must never expand the independent upload worker budget.
    #[tokio::test]
    async fn upload_pool_caps_workers_without_changing_http_budget() {
        let facts = Arc::new(Facts::default());
        let (tx, _) = mpsc::unbounded_channel();
        let pool = spawn_upload_pool_with_feedback(
            40,
            Arc::new(IdleUploader),
            FeedbackSink::Raw(tx),
            Overflow::Block,
            facts.clone(),
        );
        pool.shutdown().await;
        let records = facts.0.lock().unwrap();
        assert!(records
            .iter()
            .any(|event| event.id == MetricId::PoolWorkers && event.value == 6.0));
    }
}
