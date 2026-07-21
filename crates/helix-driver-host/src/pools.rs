//! Persist / Http BoundedSpawner 构造助手（泛型 over Storage / HttpRequester）。
//!
//! - Persist：per-table-set 懒建，每实例 N=1（保写序），Block 溢出。
//! - Http(必达)：全局单例 N=max_http_inflight，Block 溢出。
//! - HttpFire(可丢)：全局单例 N，DropNewest 溢出（满即丢当前条目）。

use std::collections::HashMap;
use std::sync::Arc;

use bytes::Bytes;
use serde_json::json;
use tokio::sync::mpsc;

use helix_core::effect::{FileUploadRequest, HttpRequest, StorageOp};
use helix_core::ports::{FileUploader, HttpRequester, Storage};
use helix_core::tick::{PortOutcome, ReplyBytes};
use helix_core::Tick;

use crate::http_envelope::run_http_envelope;
use crate::ops::classify_port_error;
use crate::spawner::{BoundedSpawner, Overflow};
use crate::storage::{with_storage_trace_context, StorageTraceContext};
use crate::{execute_storage_ops, execute_storage_ops_atomic, PERSIST_QUEUE_CAP};

use crate::HTTP_QUEUE_CAP;

pub struct PersistJobPayload {
    pub ops: Vec<StorageOp>,
    /// `Effect::PersistAtomic` opt-in; ordinary Persist/PersistFire retain their existing semantics.
    pub atomic: bool,
    pub trace: Option<StorageTraceContext>,
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
            move |payload: PersistJobPayload| {
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
        move |req: HttpRequest| {
            let http = Arc::clone(&http);
            async move { run_http_envelope(http.as_ref(), req).await }
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
    BoundedSpawner::new(
        concurrency,
        HTTP_QUEUE_CAP,
        overflow,
        reply_tx,
        move |req: FileUploadRequest| {
            let uploader = Arc::clone(&uploader);
            async move {
                match uploader.upload(req).await {
                    Ok(resp) => {
                        let bytes = serde_json::to_vec(&json!({
                            "objectKey": resp.object_key,
                            // PortReply wire 保留兼容键 `url`，值只来自长期可读地址。
                            "url": resp.public_url,
                            "etag": resp.etag,
                        }))
                        .expect("upload response JSON serialization cannot fail");
                        PortOutcome::Ok(ReplyBytes(Bytes::from(bytes)))
                    }
                    Err(e) => PortOutcome::Err(classify_port_error(e)),
                }
            }
        },
    )
}
