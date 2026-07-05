//! dispatch — 一批 OwnedEffect 的分流兑现（泵核子步骤，从 engine 拆出）。
//!
//! 泵线程绝不在此 await worker 完成（抽水在独立 worker，submit 即返回；死锁红线，不变量2）。
//! Persist/Http 走 BoundedSpawner submit（Block 满时背压传导回 tick_tx）；
//! Send inline await（WS send 快）；Emit/Timer 同步；Request 当前 deferred stub。

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::mpsc;

use helix_core::effect::{HttpRequest, StorageOp};
use helix_core::ports::{EventSink, Storage, Transport};
use helix_core::Tick;

use crate::engine::TransportTable;
use crate::owned_effect::OwnedEffect;
use crate::pools::get_or_spawn_persist;
use crate::spawner::{BoundedSpawner, Job};
use crate::table_set_key;
use crate::timer::TimerRegistry;
use crate::trace::TraceHooks;

/// 把一批 OwnedEffect 分流兑现（async，BoundedSpawner submit，Block 满时等待）。
#[allow(clippy::too_many_arguments)]
pub(crate) async fn dispatch_effects<S, E, Tr>(
    effects: Vec<OwnedEffect>,
    tick_tx: &mpsc::Sender<Tick>,
    storage: &Arc<S>,
    http_pool: &BoundedSpawner<HttpRequest>,
    http_fire_pool: &BoundedSpawner<HttpRequest>,
    event_sink: &Arc<E>,
    timer_registry: &mut TimerRegistry,
    reply_tx: &mpsc::UnboundedSender<Tick>,
    persist_workers: &mut HashMap<String, BoundedSpawner<Vec<StorageOp>>>,
    transports: &TransportTable<Tr>,
    trace: &TraceHooks,
) where
    S: Storage + Send + Sync + 'static,
    E: EventSink,
    Tr: Transport + Send + 'static,
{
    for effect in effects {
        match effect {
            // ── Persist（带 corr，写完后通过 reply_tx 回报）────────────────
            OwnedEffect::Persist { corr, ops } => {
                trace.on_storage_dispatch(Some(corr), &ops);
                let key = table_set_key(&ops);
                let sp = get_or_spawn_persist(
                    persist_workers,
                    key,
                    Arc::clone(storage),
                    reply_tx.clone(),
                );
                // Block submit：队列满时 .await 阻塞主循环，背压传导到 tick_tx（不变量2）。
                sp.submit(Job {
                    corr: Some(corr),
                    payload: ops,
                })
                .await;
            }

            // ── PersistFire（fire-and-forget，不产出 PortReply）────────────
            OwnedEffect::PersistFire { ops } => {
                trace.on_storage_dispatch(None, &ops);
                let key = table_set_key(&ops);
                let sp = get_or_spawn_persist(
                    persist_workers,
                    key,
                    Arc::clone(storage),
                    reply_tx.clone(),
                );
                sp.submit(Job {
                    corr: None,
                    payload: ops,
                })
                .await;
            }

            // ── Http（必达，Block 溢出，带 corr 回报，走 reply_tx）──────────
            OwnedEffect::Http { corr, mut req } => {
                trace.on_http_dispatch(Some(corr), &mut req);
                http_pool
                    .submit(Job {
                        corr: Some(corr),
                        payload: req,
                    })
                    .await;
            }

            // ── HttpFire（可丢，DropNewest 溢出，无 corr 不回报）────────────
            OwnedEffect::HttpFire { mut req } => {
                trace.on_http_dispatch(None, &mut req);
                http_fire_pool
                    .submit(Job {
                        corr: None,
                        payload: req,
                    })
                    .await;
            }

            // ── Send（查路由表 → Transport::send 真发 WS）──────────────────
            // inline await（WS send 快）。失败/未注册只 warn，绝不让泵崩。
            OwnedEffect::Send {
                transport,
                mut frame,
            } => match transports.get(&transport) {
                Some(t) => {
                    trace.on_ws_send(transport, &mut frame);
                    // 表内 `Arc<Tr>`（无外层锁）→ 直接 `t.send()`（`&self`）。并发 send 由
                    // transport 内部 state 锁串行化（既有锁，非本路径新增 per-Send 锁，守 HX-C005）。
                    if let Err(e) = t.send(frame).await {
                        tracing::warn!(
                            transport_id = transport.raw(),
                            error = %e,
                            "Effect::Send 发送失败（连接异常，待重连/补偿兜底）"
                        );
                    }
                }
                None => {
                    tracing::warn!(
                        transport_id = transport.raw(),
                        frame_len = frame.len(),
                        "Effect::Send 无对应 transport（路由表未注册该 id）"
                    );
                }
            },

            // ── Request（EventBus REP·方向①：发起端）─────────────────────
            // TODO(方向①收尾): 接入 HostRequester port。当前为 deferred stub——仅日志、不回灌。
            OwnedEffect::Request {
                corr,
                kind,
                payload,
            } => {
                tracing::warn!(
                    corr = corr.raw(),
                    kind,
                    payload_len = payload.len(),
                    "Effect::Request 尚未接入 HostRequester port（方向① deferred）"
                );
            }

            // ── Emit（同步非阻塞）─────────────────────────────────────────
            OwnedEffect::Emit { event } => {
                trace.on_event_emit(&event);
                event_sink.emit(event);
            }

            // ── ScheduleTimer（timer fires → 往 tick_tx 有界发送）─────────
            OwnedEffect::ScheduleTimer { id, after_ms } => {
                timer_registry.schedule(id, after_ms, tick_tx.clone());
            }

            // ── CancelTimer（幂等）────────────────────────────────────────
            OwnedEffect::CancelTimer { id } => {
                timer_registry.cancel(id);
            }
        }
    }
}
