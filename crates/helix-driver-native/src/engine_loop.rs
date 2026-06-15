//! engine_loop — tokio 事件泵
//!
//! ## tokio::spawn 在此，不在 helix-core
//!
//! helix-core 保持零运行时依赖；所有异步协调（WS recv、HTTP 响应、timer fired、
//! AppCommand）在此 tokio mpsc 统一入站。
//!
//! ## 架构（E1 修复 + review 回炉后）
//!
//! ```text
//! +- tick_rx (mpsc, cap=256) ─────────────────────────────────────────────────+
//! |  WS reads (per transport) -> Tick::Inbound / Connected / Disconnected      |
//! |  timer spawn             -> Tick::Timer                                     |
//! |  AppCommand (external)   -> Tick::Command                                  |
//! +───────────────────────────────────────────────────────────────────────────+
//!                v  biased select（reply_rx 优先）
//!     ExecutionShell::step(tick, now_ms)
//!                v &[Effect] (borrows shell.scratch, valid until next step)
//!     consume_effects() -> collect to OwnedEffect (frees borrow)
//!     dispatch_owned_effects() -> dispatch by type，立即返回不等磁盘
//!                                  Persist → 入 per-table-set 有序 worker 队列
//!
//! +- reply_tx (unbounded mpsc) ─────────────────────────────────────────────+
//! |  per-table worker tasks  -> Tick::PortReply（写完即回灌，不等引擎收）     |
//! |  http spawn              -> Tick::PortReply                               |
//! +─────────────────────────────────────────────────────────────────────────+
//! ```
//!
//! ## E1 死锁根因与修复
//!
//! 原实现：PortReply 回灌与入站 Tick 共用同一条有界 mpsc(cap=256)，且
//! dispatch_owned_effects 末尾 await 全部 persist handles → 洪峰 >256 环形等待。
//!
//! ## 修复四不变量（review 回炉校正版）
//!
//! 1. **回灌必达**：PortReply 走独立 unbounded channel（reply_tx），永不堵塞写任务。
//!    reply_rx 中排队的条目 ≤ 已完成 job 数（有界上界）；真正有界的约束在入站背压——
//!    **入站背压仍是接入方契约**：调用方必须使用有界 tick_tx（cap=256）并在背压下
//!    等待，避免 job queue 积压无界增长。per-table job queue 容量 = `PERSIST_QUEUE_CAP`
//!    （有界），超限时 dispatch 侧 `.await` 阻塞主循环，把背压传导回 tick_tx。
//! 2. **主循环不等磁盘**：dispatch 把 Persist 入队到 per-table worker 后立即返回
//!    （job queue 未满时）；满时阻塞主循环（有界背压传导），不会无界积压。
//! 3. **同组写顺序保持**：routing key = ops 涉及的全部 table 名（有序集合，"|" 分隔）。
//!    单表 ops → 单表 key（常见路径，无额外开销）；
//!    多表 ops → 组合 key（退化到一个独立 worker，不与单表 worker 交叉），
//!    保证跨 tick 的同 key 写严格按产生顺序执行。
//!    **约束**：调用方需保证「需要相互有序的 Persist 使用相同 table-set key」。
//!    若 tick1 的 `Persist{ops:[op_A]}` 和 tick2 的 `Persist{ops:[op_A, op_B]}` 需要
//!    全局有序，应合并为同一 Persist 或在 core 层保证单表。见 `table_set_key` 文档。
//! 4. **回灌优先**：biased select! 把 reply_rx 排在 tick_rx 前，防 PortReply 饥饿。
//! 5. **退出 drain**：tick_rx 关闭后，drop 全部 job sender，join 全部 worker task，
//!    drain reply_rx，确保「run_engine_loop 返回 = 全部已接收 Persist 均已落盘」。
//!
//! ## Effect 借用收集策略（关键）
//!
//! `ExecutionShell::step()` 返回 `&[Effect]`，借用 `shell.scratch`（跨 step 复用 Vec）。
//! 下次 `step()` 前必须消费完。策略：立即 `consume_effects()` 把 `&[Effect]` 转成
//! `Vec<OwnedEffect>`（owned struct，按需 Clone 字段），脱离借用后再 async 兑现。
//!
//! ## Effect 分流策略
//!
//! - `Persist` / `PersistFire`：按目标表名分组，同组顺序执行，跨组 tokio::spawn 并发（最大 4）
//! - `Http`：tokio::spawn 并发（受 reqwest 客户端连接池自限）
//! - `Send`：tracing 记录（生产接入时调用者传入 Transport map）
//! - `Emit`：同步调用 EventSink::emit
//! - `ScheduleTimer` / `CancelTimer`：TimerRegistry 操作
//!
//! ## rows→bytes 序列化约定
//!
//! `scan` 返回 `Vec<Row>`，driver 统一用 JSON 序列化进 ReplyBytes：
//! `[{"col1": val1, ...}, ...]`，SqlValue 映射：
//!   - Text(s)    → `"s"` (JSON string)
//!   - Integer(i) → `i`  (JSON number)
//!   - Real(f)    → `f`  (JSON number)
//!   - Null       → `null`
//! 反序列化时按 JSON 类型还原 SqlValue（整数 → Integer，浮点 → Real）。

use std::collections::HashMap;
use std::sync::Arc;

use bytes::Bytes;
use serde_json::json;
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;

use helix_core::effect::{
    Correlation, DomainEventBytes, Effect, HttpRequest, StorageOp, TimerId, TransportId,
};
use helix_core::ports::{Clock, EventSink, HttpRequester};
use helix_core::tick::{PortOutcome, ReplyBytes};
use helix_core::{ExecutionShell, Tick};
use helix_driver_host::{classify_port_error, execute_storage_ops, table_set_key};
pub use helix_driver_host::{rows_from_reply_bytes, rows_to_reply_bytes};

use crate::clock::NativeClock;
use crate::event_sink::NativeEventSink;
use crate::http::NativeHttp;
use crate::storage::NativeStorage;
use crate::timer::TimerRegistry;

/// per-table worker 的 job 队列深度上限（有界背压）。
///
/// 超过此值时 dispatch 侧 `.await` 阻塞主循环，将背压传导到 tick_tx（cap=256）。
/// 这恢复了旧实现"磁盘慢则入站慢"的背压链，同时避免死锁（PortReply 走独立通道）。
const PERSIST_QUEUE_CAP: usize = 1024;

/// 事件泵配置（所有依赖项聚合）。
pub struct EngineConfig {
    pub storage: NativeStorage,
    pub http: NativeHttp,
    pub event_sink: NativeEventSink,
}

// ─── OwnedEffect ─────────────────────────────────────────────────────────────
//
// `Effect` 自身不实现 Clone（持有 non-Clone Bytes 和 Vec），
// 但 `bytes::Bytes` 是 O(1) clone（引用计数），其他字段都可 clone。
// 此 owned 镜像立即从 `&[Effect]` 转换，脱离 shell.scratch 借用。

enum OwnedEffect {
    Send {
        transport: TransportId,
        frame: Bytes,
    },
    Persist {
        corr: Correlation,
        ops: Vec<StorageOp>,
    },
    PersistFire {
        ops: Vec<StorageOp>,
    },
    Http {
        corr: Correlation,
        req: HttpRequest,
    },
    Emit {
        event: DomainEventBytes,
    },
    ScheduleTimer {
        id: TimerId,
        after_ms: u64,
    },
    CancelTimer {
        id: TimerId,
    },
}

/// 立即从 `&[Effect]` 转换为 `Vec<OwnedEffect>`（脱离借用）。
///
/// `bytes::Bytes` clone 是 O(1) 引用计数；`Vec` clone 按需复制。
/// 此函数同步执行，在 shell 返回 `&[Effect]` 后立即调用。
fn consume_effects(effects: &[Effect]) -> Vec<OwnedEffect> {
    effects
        .iter()
        .map(|e| match e {
            Effect::Send { transport, frame } => OwnedEffect::Send {
                transport: *transport,
                frame: frame.clone(),
            },
            Effect::Persist { corr, ops } => OwnedEffect::Persist {
                corr: *corr,
                ops: ops.clone(),
            },
            Effect::PersistFire { ops } => OwnedEffect::PersistFire { ops: ops.clone() },
            Effect::Http { corr, req } => OwnedEffect::Http {
                corr: *corr,
                req: req.clone(),
            },
            Effect::Emit { event } => OwnedEffect::Emit {
                event: event.clone(),
            },
            Effect::ScheduleTimer { id, after_ms } => OwnedEffect::ScheduleTimer {
                id: *id,
                after_ms: *after_ms,
            },
            Effect::CancelTimer { id } => OwnedEffect::CancelTimer { id: *id },
        })
        .collect()
}

// ─── Per-table-set 有序 Worker ───────────────────────────────────────────────

/// 一条进 per-table-set worker 队列的工作单元。
type PersistJob = (Option<Correlation>, Vec<StorageOp>);

/// per-table-set worker 的句柄（发送端 + JoinHandle，用于 graceful drain）。
struct WorkerHandle {
    /// job 发送端（bounded channel，drop 时 worker 任务自然退出）。
    tx: mpsc::Sender<PersistJob>,
    /// worker task 句柄，shutdown 时 join 确保积压全部落盘。
    join: JoinHandle<()>,
}

/// 启动（或复用）某 table-set 对应的长驻有序 worker task，返回其发送端引用。
///
/// ## 设计不变量（review 回炉 §3）
///
/// 同 key 内所有 Persist/PersistFire 严格按入队顺序执行，worker task 是单消费者。
/// job queue 有界（`PERSIST_QUEUE_CAP`），超限时 caller .await 阻塞，背压传导到
/// tick_tx（有界入站通道）。
/// 写完后若有 corr → 往 reply_tx（unbounded）发 PortReply，永不堵塞写 worker。
///
/// ## 注意：此函数是 async 的（bounded send 需 await）
///
/// 调用方（dispatch_effects）须在 async 上下文中 .await get_or_spawn_worker 的
/// send 操作，见 `enqueue_persist`。
fn get_or_spawn_worker(
    workers: &mut HashMap<String, WorkerHandle>,
    table_key: String,
    storage: Arc<NativeStorage>,
    reply_tx: mpsc::UnboundedSender<Tick>,
) -> &WorkerHandle {
    workers.entry(table_key).or_insert_with(|| {
        let (job_tx, mut job_rx) = mpsc::channel::<PersistJob>(PERSIST_QUEUE_CAP);
        let join = tokio::spawn(async move {
            while let Some((corr_opt, ops)) = job_rx.recv().await {
                let outcome = execute_storage_ops(storage.as_ref(), &ops).await;
                if let Some(corr) = corr_opt {
                    // unbounded send：不会阻塞，回灌必达（§1）
                    reply_tx.send(Tick::PortReply { corr, outcome }).ok();
                }
                // PersistFire (corr_opt = None) → fire-and-forget，不回报
            }
        });
        WorkerHandle { tx: job_tx, join }
    })
}

// ─── engine loop ─────────────────────────────────────────────────────────────

/// 运行 ExecutionShell 的 tokio 事件泵。
///
/// ## 使用方式
///
/// ```rust,ignore
/// let (tick_tx, tick_rx) = mpsc::channel::<Tick>(256);
/// let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
/// let config = EngineConfig { storage, http, event_sink };
/// tokio::spawn(run_engine_loop(shell, tick_rx, tick_tx.clone(), config, shutdown_rx));
/// // tick_tx 克隆给外部（Tauri command handler / Transport 读任务 / 测试）注入 Tick
/// // shutdown_tx.send(()) 触发 graceful drain + 退出
/// ```
///
/// ## Shutdown 语义
///
/// 发送 `shutdown_tx.send(())` 后，引擎停止接收新 Tick，执行 graceful drain：
/// 等待全部已入队 Persist 落盘后返回。
/// 返回 = 「全部已接收 Persist 均已落盘」。
pub async fn run_engine_loop(
    mut shell: ExecutionShell,
    mut tick_rx: mpsc::Receiver<Tick>,
    tick_tx: mpsc::Sender<Tick>,
    config: EngineConfig,
    mut shutdown_rx: oneshot::Receiver<()>,
) {
    let clock = NativeClock;
    let storage = Arc::new(config.storage);
    let http = Arc::new(config.http);
    let event_sink = Arc::new(config.event_sink);
    let mut timer_registry = TimerRegistry::new();

    // E1 修复：独立 unbounded 回灌通道（不与入站 tick_rx 共用，永不堵塞写任务）
    let (reply_tx, mut reply_rx) = mpsc::unbounded_channel::<Tick>();

    // per-table-set 长驻 worker 映射（table_set_key → WorkerHandle{tx, join}）
    let mut persist_workers: HashMap<String, WorkerHandle> = HashMap::new();

    // 初始化：触发 on_start
    let initial_effects = match shell.start() {
        Ok(effects) => consume_effects(effects),
        Err(e) => {
            tracing::error!("helix engine start failed: {}", e);
            return;
        }
    };
    tracing::info!(
        "helix engine started, {} initial effects",
        initial_effects.len()
    );

    dispatch_effects(
        initial_effects,
        &tick_tx,
        &storage,
        &http,
        &event_sink,
        &mut timer_registry,
        &reply_tx,
        &mut persist_workers,
    )
    .await;

    // 事件泵主循环
    //
    // biased select!：reply_rx 排在前 → PortReply 优先于新入站 Tick（§4）。
    // 若 reply_rx 有待处理回灌，优先消费，防 PortReply 在洪峰时饥饿。
    //
    // shutdown_armed：oneshot 只允许被 poll 到完成一次（poll-after-completion 会
    // panic），完成后该分支永久解除武装。
    let mut shutdown_armed = true;
    loop {
        let tick = tokio::select! {
            biased;

            // 优先：PortReply 回灌
            Some(reply) = reply_rx.recv() => reply,

            // 次选：shutdown 信号（graceful drain 入口）
            // Ok(()) = 调用方显式 send(()) → 立即进入 graceful drain；
            // Err(_) = sender 被 drop 而未 send → 视为「无 shutdown 信号」继续运行
            //          （此时唯一退出路径是 tick 通道关闭）。
            // shutdown_armed 防 poll-after-completion：tokio oneshot 完成后再 poll
            // 会 panic——若只写 `Ok(()) = ...`，Err 完成时分支不命中、下一轮 select
            // 重新 poll 已完成的 oneshot 即触发 panic。
            res = &mut shutdown_rx, if shutdown_armed => {
                shutdown_armed = false;
                if res.is_ok() { break; }
                continue;
            }

            // 三选：入站 Tick（Command / Inbound / Timer / …）
            tick = tick_rx.recv() => {
                match tick {
                    Some(t) => t,
                    None => break, // tick 通道关闭，进入 graceful drain
                }
            }
        };

        let now_ms = clock.now_ms();
        let owned = match shell.step(tick, now_ms) {
            Ok(effects) => consume_effects(effects),
            Err(e) => {
                tracing::error!("engine step error: {}", e);
                continue;
            }
        };

        if !owned.is_empty() {
            dispatch_effects(
                owned,
                &tick_tx,
                &storage,
                &http,
                &event_sink,
                &mut timer_registry,
                &reply_tx,
                &mut persist_workers,
            )
            .await;
        }
    }

    // ── Graceful drain（§5，review 回炉）────────────────────────────────────
    //
    // tick_rx 关闭 → 不再有新 Tick 进来。此时 per-table worker 可能还有积压 job。
    //
    // 正确关闭顺序：
    // 1. drop 全部 job sender（worker 排空队列后自然退出）
    // 2. join 全部 worker task（保证积压 job 全部落盘）
    // 3. drain reply_rx（捡走写完后才回灌的最后一批 PortReply，避免 log 噪音）
    //
    // 保证：run_engine_loop 返回 = 全部已接收 Persist 均已落盘。
    // 破坏此保证的改动需同步更新本文档注释。
    tracing::info!(
        "helix engine loop: tick_rx closed, draining {} workers",
        persist_workers.len()
    );

    // step 1: 提取全部 JoinHandle，同时 drop 全部 job sender（Sender drop → worker 的 recv 返回 None）
    let join_handles: Vec<JoinHandle<()>> = persist_workers
        .drain()
        .map(|(_, wh)| {
            drop(wh.tx); // drop sender，触发 worker job_rx.recv() 返回 None
            wh.join
        })
        .collect();

    // step 2: join 全部 worker（等待积压 job 全部落盘）
    for handle in join_handles {
        if let Err(e) = handle.await {
            tracing::error!("persist worker panicked during shutdown: {:?}", e);
        }
    }

    // step 3: drain reply_rx（write worker 写完最后 job 发出的 PortReply）
    while reply_rx.try_recv().is_ok() {}

    tracing::info!("helix engine loop exited (all persists drained)");
}

/// 把一批 OwnedEffect 分流兑现（async，Persist 入有界队列，满时等待）。
///
/// ## review 回炉要点
///
/// - Persist / PersistFire：按 `table_set_key` 路由到对应 worker，`.send().await`
///   在队列满时阻塞主循环，将背压传导到 tick_tx，避免 job queue 无界增长（§1 / §2）。
/// - PortReply 回灌走 reply_tx（unbounded），由 worker 任务独立发送（§1）。
async fn dispatch_effects(
    effects: Vec<OwnedEffect>,
    tick_tx: &mpsc::Sender<Tick>,
    storage: &Arc<NativeStorage>,
    http: &Arc<NativeHttp>,
    event_sink: &Arc<NativeEventSink>,
    timer_registry: &mut TimerRegistry,
    reply_tx: &mpsc::UnboundedSender<Tick>,
    persist_workers: &mut HashMap<String, WorkerHandle>,
) {
    for effect in effects {
        match effect {
            // ── Persist（带 corr，写完后通过 reply_tx 回报）────────────────
            OwnedEffect::Persist { corr, ops } => {
                let key = table_set_key(&ops);
                let wh = get_or_spawn_worker(
                    persist_workers,
                    key,
                    Arc::clone(storage),
                    reply_tx.clone(),
                );
                // bounded send：队列满时 .await 阻塞主循环，背压传导到 tick_tx
                wh.tx.send((Some(corr), ops)).await.ok();
            }

            // ── PersistFire（fire-and-forget，不产出 PortReply）────────────
            OwnedEffect::PersistFire { ops } => {
                let key = table_set_key(&ops);
                let wh = get_or_spawn_worker(
                    persist_workers,
                    key,
                    Arc::clone(storage),
                    reply_tx.clone(),
                );
                // bounded send：背压传导同上
                wh.tx.send((None, ops)).await.ok();
            }

            // ── Http（并发，带 corr 回报，走 reply_tx）────────────────────
            OwnedEffect::Http { corr, req } => {
                let http_clone = Arc::clone(http);
                let reply_clone = reply_tx.clone();
                tokio::spawn(async move {
                    let outcome = match http_clone.request(req).await {
                        Ok(resp) => {
                            let body_b64 = base64_encode(&resp.body);
                            let json_bytes = serde_json::to_vec(&json!({
                                "status": resp.status,
                                "headers": resp.headers,
                                "body": body_b64,
                            }))
                            .unwrap_or_default();
                            PortOutcome::Ok(ReplyBytes(Bytes::from(json_bytes)))
                        }
                        Err(e) => PortOutcome::Err(classify_port_error(e)),
                    };
                    reply_clone.send(Tick::PortReply { corr, outcome }).ok();
                });
            }

            // ── Send（当前泵内记录，生产接入时替换为 Transport::send）────
            OwnedEffect::Send { transport, frame } => {
                tracing::debug!(
                    transport_id = transport.raw(),
                    frame_len = frame.len(),
                    "Effect::Send — wire Transport into engine_loop for production"
                );
            }

            // ── Emit（同步非阻塞）─────────────────────────────────────────
            OwnedEffect::Emit { event } => {
                event_sink.emit(event);
            }

            // ── ScheduleTimer ──────────────────────────────────────────────
            OwnedEffect::ScheduleTimer { id, after_ms } => {
                // timer fires → 往 tick_tx（有界）发送；timer 回调走入站通道
                timer_registry.schedule(id, after_ms, tick_tx.clone());
            }

            // ── CancelTimer（幂等）────────────────────────────────────────
            OwnedEffect::CancelTimer { id } => {
                timer_registry.cancel(id);
            }
        }
    }
}

// ─── 辅助函数 ────────────────────────────────────────────────────────────────

/// 简单 base64 编码（HTTP body 放进 JSON 时用，避免 binary 污染 JSON string）。
fn base64_encode(bytes: &[u8]) -> String {
    const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity((bytes.len() + 2) / 3 * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(TABLE[((n >> 18) & 63) as usize] as char);
        out.push(TABLE[((n >> 12) & 63) as usize] as char);
        out.push(if chunk.len() > 1 {
            TABLE[((n >> 6) & 63) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            TABLE[(n & 63) as usize] as char
        } else {
            '='
        });
    }
    out
}

// ─── 单元测试 ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use helix_core::effect::SqlValue;
    use helix_core::tick::PortError as CorePortError;
    use helix_core::PortError;

    #[test]
    fn test_rows_roundtrip() {
        let rows = vec![vec![
            ("id".to_string(), SqlValue::Integer(1)),
            ("name".to_string(), SqlValue::Text("alice".to_string())),
            ("score".to_string(), SqlValue::Real(9.5)),
            ("extra".to_string(), SqlValue::Null),
        ]];
        let bytes = rows_to_reply_bytes(&rows);
        let restored = rows_from_reply_bytes(&bytes).expect("roundtrip");
        assert_eq!(restored.len(), 1);
        let row = &restored[0];
        assert!(matches!(
            row.iter().find(|(k, _)| k == "id").map(|(_, v)| v),
            Some(SqlValue::Integer(1))
        ));
        assert!(matches!(
            row.iter().find(|(k, _)| k == "name").map(|(_, v)| v),
            Some(SqlValue::Text(s)) if s == "alice"
        ));
        assert!(matches!(
            row.iter().find(|(k, _)| k == "extra").map(|(_, v)| v),
            Some(SqlValue::Null)
        ));
    }

    #[test]
    fn test_rows_to_bytes_empty() {
        let bytes = rows_to_reply_bytes(&[]);
        assert_eq!(&bytes[..], b"[]");
    }

    #[test]
    fn test_classify_port_error_timeout() {
        let e = PortError::Transport("timeout: connection timed out".to_string());
        assert_eq!(classify_port_error(e), CorePortError::Timeout);
    }

    #[test]
    fn test_classify_port_error_network() {
        let e = PortError::Transport("network: connection refused".to_string());
        assert_eq!(classify_port_error(e), CorePortError::Network);
    }

    #[test]
    fn test_classify_port_error_storage() {
        let e = PortError::Storage("disk full".to_string());
        assert!(matches!(classify_port_error(e), CorePortError::Storage(_)));
    }

    #[test]
    fn test_base64_encode() {
        assert_eq!(base64_encode(b"Man"), "TWFu");
        assert_eq!(base64_encode(b"Ma"), "TWE=");
        assert_eq!(base64_encode(b"M"), "TQ==");
        assert_eq!(base64_encode(b""), "");
    }

    #[test]
    fn test_table_set_key_empty() {
        assert_eq!(table_set_key(&[]), "");
    }

    #[test]
    fn test_table_set_key_single_table() {
        use helix_core::effect::UpsertSpec;
        let ops = vec![StorageOp::BatchUpsert(UpsertSpec {
            table: "messages",
            rows: vec![],
            conflict_key: None,
        })];
        assert_eq!(table_set_key(&ops), "messages");
    }

    #[test]
    fn test_table_set_key_multi_table_sorted() {
        use helix_core::effect::{MonotonicUpsertSpec, UpsertSpec};
        // 两个不同 table 的 ops，顺序无所谓，键必须按字典序排
        let ops = vec![
            StorageOp::BatchUpsert(UpsertSpec {
                table: "messages",
                rows: vec![],
                conflict_key: None,
            }),
            StorageOp::MonotonicUpsert(MonotonicUpsertSpec {
                scope_key: "ch:1".to_string(),
                value: 1,
            }),
        ];
        // messages > _helix_monotonic（字典序），键应是 "_helix_monotonic|messages"
        assert_eq!(table_set_key(&ops), "_helix_monotonic|messages");
    }

    #[test]
    fn test_table_set_key_dedup_same_table() {
        use helix_core::effect::UpsertSpec;
        // 同一 table 出现多次，去重后仍是单 key
        let ops = vec![
            StorageOp::BatchUpsert(UpsertSpec {
                table: "messages",
                rows: vec![],
                conflict_key: None,
            }),
            StorageOp::BatchUpsert(UpsertSpec {
                table: "messages",
                rows: vec![],
                conflict_key: None,
            }),
        ];
        assert_eq!(table_set_key(&ops), "messages");
    }
}
