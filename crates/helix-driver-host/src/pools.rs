//! Persist / Http BoundedSpawner 构造助手（泛型 over Storage / HttpRequester）。
//!
//! - Persist：per-table-set 懒建，每实例 N=1（保写序），Block 溢出。
//! - Http(必达)：全局单例 N=max_http_inflight，Block 溢出。
//! - HttpFire(可丢)：全局单例 N，DropNewest 溢出（满即丢当前条目）。

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::mpsc;

use helix_core::effect::{HttpRequest, StorageOp};
use helix_core::ports::{HttpRequester, Storage};
use helix_core::Tick;

use crate::http_envelope::run_http_envelope;
use crate::spawner::{BoundedSpawner, Overflow};
use crate::{execute_storage_ops, PERSIST_QUEUE_CAP};

use crate::HTTP_QUEUE_CAP;

/// 启动（或复用）某 table-set 对应的 Persist BoundedSpawner（N=1 单消费者保序）。
///
/// ## 设计不变量（不变量3）
///
/// 同 key 内所有 Persist/PersistFire 严格按入队顺序执行（N=1 → 单 worker 单消费者）。
/// job queue 有界（`PERSIST_QUEUE_CAP`），超限时 caller `.await` 阻塞，背压传导到 tick_tx。
pub fn get_or_spawn_persist<S>(
    workers: &mut HashMap<String, BoundedSpawner<Vec<StorageOp>>>,
    table_key: String,
    storage: Arc<S>,
    reply_tx: mpsc::UnboundedSender<Tick>,
) -> &BoundedSpawner<Vec<StorageOp>>
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
            move |ops: Vec<StorageOp>| {
                let storage = Arc::clone(&storage);
                async move { execute_storage_ops(storage.as_ref(), &ops).await }
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
