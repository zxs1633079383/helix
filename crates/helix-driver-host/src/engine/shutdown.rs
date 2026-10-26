use std::collections::HashMap;
use std::time::Duration;

use helix_core::effect::{FileUploadRequest, HttpRequest};
use tokio::sync::mpsc;

use crate::pools::PersistJobPayload;
use crate::spawner::{BoundedSpawner, StampedFeedback};

const HTTP_DRAIN_LIMIT: Duration = Duration::from_secs(3);

pub(super) async fn drain_pools(
    mut persist_workers: HashMap<String, BoundedSpawner<PersistJobPayload>>,
    upload_pool: BoundedSpawner<FileUploadRequest>,
    http_pool: BoundedSpawner<HttpRequest>,
    http_fire_pool: BoundedSpawner<HttpRequest>,
    mut reply_rx: mpsc::UnboundedReceiver<StampedFeedback>,
) {
    tracing::info!(
        "helix engine loop: tick_rx closed, draining {} persist + io pools",
        persist_workers.len()
    );
    for (_, spawner) in persist_workers.drain() {
        spawner.shutdown().await;
    }
    if !upload_pool.shutdown_with_timeout(HTTP_DRAIN_LIMIT).await {
        tracing::warn!(
            limit_ms = HTTP_DRAIN_LIMIT.as_millis(),
            "UploadFile drain 超上限——abort 在途上传任务"
        );
    }
    if !http_pool.shutdown_with_timeout(HTTP_DRAIN_LIMIT).await {
        tracing::warn!(
            limit_ms = HTTP_DRAIN_LIMIT.as_millis(),
            "必达 Http drain 超上限——abort 在途请求（乐观态已持久化，靠重连重发对账兜底）"
        );
    }
    let _ = http_fire_pool.shutdown_with_timeout(HTTP_DRAIN_LIMIT).await;
    while reply_rx.try_recv().is_ok() {}
}
