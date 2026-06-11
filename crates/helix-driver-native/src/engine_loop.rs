//! engine_loop — tokio 事件泵
//!
//! ## tokio::spawn 在此，不在 helix-core
//!
//! helix-core 保持零运行时依赖；所有异步协调（WS recv、HTTP 响应、timer fired、
//! AppCommand）在此 tokio mpsc 统一入站。
//!
//! ## 架构（E1 修复后）
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
//!                                  Persist → 入 per-table 有序 worker 队列
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
//! 修复四不变量：
//! 1. **回灌必达**：PortReply 走独立 unbounded channel（reply_tx），永不堵塞写任务。
//!    in-flight 数受 corr_map 登记数自然有界，unbounded 不会无限增长。
//! 2. **主循环不等磁盘**：dispatch 把 Persist 分发进 per-table worker 队列后立即返回，
//!    不 await 落库完成，主循环继续收下一帧。
//! 3. **同组写顺序保持**：每个 table 对应一个长驻 worker task + 自己的有序队列
//!    (tokio::mpsc, unbounded)；跨 tick 的同组 Persist 严格按产生顺序执行。
//! 4. **回灌优先**：biased select! 把 reply_rx 排在 tick_rx 前，防 PortReply 饥饿。
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
use serde_json::{json, Value as JsonValue};
use tokio::sync::mpsc;

use helix_core::{ExecutionShell, Tick};
use helix_core::effect::{Effect, StorageOp, Correlation, TimerId, TransportId,
                          HttpRequest, DomainEventBytes, SqlValue};
use helix_core::tick::{PortOutcome, ReplyBytes, PortError as CorePortError};
use helix_core::PortError;
use helix_core::ports::{Storage, HttpRequester, EventSink, Clock};

use crate::clock::NativeClock;
use crate::storage::NativeStorage;
use crate::http::NativeHttp;
use crate::event_sink::NativeEventSink;
use crate::timer::TimerRegistry;

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
    effects.iter().map(|e| match e {
        Effect::Send { transport, frame } => OwnedEffect::Send {
            transport: *transport,
            frame: frame.clone(),
        },
        Effect::Persist { corr, ops } => OwnedEffect::Persist {
            corr: *corr,
            ops: ops.clone(),
        },
        Effect::PersistFire { ops } => OwnedEffect::PersistFire {
            ops: ops.clone(),
        },
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
        Effect::CancelTimer { id } => OwnedEffect::CancelTimer {
            id: *id,
        },
    }).collect()
}

// ─── Per-table 有序 Worker ────────────────────────────────────────────────────

/// 一条进 per-table worker 队列的工作单元。
type PersistJob = (Option<Correlation>, Vec<StorageOp>);

/// 启动（或复用）某 table 对应的长驻有序 worker task，返回其发送端。
///
/// ## 设计不变量（E1 修复 §3）
///
/// 同组内所有 Persist/PersistFire 严格按入队顺序执行，worker task 是单消费者。
/// 写完后若有 corr → 往 reply_tx（unbounded）发 PortReply，不因队列满堵塞。
fn get_or_spawn_worker(
    workers: &mut HashMap<String, mpsc::UnboundedSender<PersistJob>>,
    table_key: String,
    storage: Arc<NativeStorage>,
    reply_tx: mpsc::UnboundedSender<Tick>,
) -> &mpsc::UnboundedSender<PersistJob> {
    workers.entry(table_key).or_insert_with(|| {
        let (job_tx, mut job_rx) = mpsc::unbounded_channel::<PersistJob>();
        tokio::spawn(async move {
            while let Some((corr_opt, ops)) = job_rx.recv().await {
                let outcome = execute_storage_ops(&storage, &ops).await;
                if let Some(corr) = corr_opt {
                    // unbounded send：不会阻塞，回灌必达（E1 修复 §1）
                    reply_tx.send(Tick::PortReply { corr, outcome }).ok();
                }
                // PersistFire (corr_opt = None) → fire-and-forget，不回报
            }
        });
        job_tx
    })
}

// ─── engine loop ─────────────────────────────────────────────────────────────

/// 运行 ExecutionShell 的 tokio 事件泵。
///
/// ## 使用方式
///
/// ```rust,ignore
/// let (tick_tx, tick_rx) = mpsc::channel::<Tick>(256);
/// let config = EngineConfig { storage, http, event_sink };
/// tokio::spawn(run_engine_loop(shell, tick_rx, tick_tx.clone(), config));
/// // tick_tx 克隆给外部（Tauri command handler / Transport 读任务 / 测试）注入 Tick
/// ```
pub async fn run_engine_loop(
    mut shell: ExecutionShell,
    mut tick_rx: mpsc::Receiver<Tick>,
    tick_tx: mpsc::Sender<Tick>,
    config: EngineConfig,
) {
    let clock = NativeClock;
    let storage = Arc::new(config.storage);
    let http = Arc::new(config.http);
    let event_sink = Arc::new(config.event_sink);
    let mut timer_registry = TimerRegistry::new();

    // E1 修复：独立 unbounded 回灌通道（不与入站 tick_rx 共用，永不堵塞写任务）
    let (reply_tx, mut reply_rx) = mpsc::unbounded_channel::<Tick>();

    // per-table 长驻 worker 映射（table_key → job 发送端）
    let mut persist_workers: HashMap<String, mpsc::UnboundedSender<PersistJob>> = HashMap::new();

    // 初始化：触发 on_start
    let initial_effects = match shell.start() {
        Ok(effects) => consume_effects(effects),
        Err(e) => {
            tracing::error!("helix engine start failed: {}", e);
            return;
        }
    };
    tracing::info!("helix engine started, {} initial effects", initial_effects.len());

    dispatch_effects(
        initial_effects,
        &tick_tx,
        &storage,
        &http,
        &event_sink,
        &mut timer_registry,
        &reply_tx,
        &mut persist_workers,
    );

    // 事件泵主循环
    //
    // biased select!：reply_rx 排在前 → PortReply 优先于新入站 Tick（E1 修复 §4）。
    // 若 reply_rx 有待处理回灌，优先消费，防 PortReply 在洪峰时饥饿。
    loop {
        let tick = tokio::select! {
            biased;

            // 优先：PortReply 回灌
            Some(reply) = reply_rx.recv() => reply,

            // 次选：入站 Tick（Command / Inbound / Timer / …）
            tick = tick_rx.recv() => {
                match tick {
                    Some(t) => t,
                    None => break, // tick 通道关闭，引擎退出
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
            );
        }
    }

    tracing::info!("helix engine loop exited (tick channel closed)");
}

/// 把一批 OwnedEffect 分流兑现（同步，立即返回，不 await 任何 I/O）。
///
/// ## E1 修复要点
///
/// - Persist / PersistFire：入队到 per-table worker，立即返回，主循环不阻塞
/// - PortReply 回灌走 reply_tx（unbounded），由 worker 任务独立发送
fn dispatch_effects(
    effects: Vec<OwnedEffect>,
    tick_tx: &mpsc::Sender<Tick>,
    storage: &Arc<NativeStorage>,
    http: &Arc<NativeHttp>,
    event_sink: &Arc<NativeEventSink>,
    timer_registry: &mut TimerRegistry,
    reply_tx: &mpsc::UnboundedSender<Tick>,
    persist_workers: &mut HashMap<String, mpsc::UnboundedSender<PersistJob>>,
) {
    for effect in effects {
        match effect {
            // ── Persist（带 corr，写完后通过 reply_tx 回报）────────────────
            OwnedEffect::Persist { corr, ops } => {
                let key = first_table_key(&ops);
                let worker_tx = get_or_spawn_worker(
                    persist_workers,
                    key,
                    Arc::clone(storage),
                    reply_tx.clone(),
                );
                worker_tx.send((Some(corr), ops)).ok();
            }

            // ── PersistFire（fire-and-forget，不产出 PortReply）────────────
            OwnedEffect::PersistFire { ops } => {
                let key = first_table_key(&ops);
                let worker_tx = get_or_spawn_worker(
                    persist_workers,
                    key,
                    Arc::clone(storage),
                    reply_tx.clone(),
                );
                worker_tx.send((None, ops)).ok();
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

// ─── Storage ops 执行 ────────────────────────────────────────────────────────

/// 执行一批 StorageOp，返回 PortOutcome。
///
/// 同组内按顺序执行，任意一个失败则返回 Err（后续 ops 跳过）。
/// 成功时返回最后一个有意义 op 的结果（scan/get → rows JSON；其余 → 空 bytes）。
async fn execute_storage_ops(
    storage: &NativeStorage,
    ops: &[StorageOp],
) -> PortOutcome {
    let mut last_ok_bytes = Bytes::new();

    for op in ops {
        match op {
            StorageOp::BatchUpsert(spec) => {
                match storage.batch_upsert(spec.clone()).await {
                    Ok(()) => {}
                    Err(e) => return PortOutcome::Err(classify_port_error(e)),
                }
            }
            StorageOp::BatchUpdate(spec) => {
                match storage.batch_update(
                    spec.table,
                    spec.key_col,
                    &spec.key_vals,
                    spec.patch.clone(),
                ).await {
                    Ok(()) => {}
                    Err(e) => return PortOutcome::Err(classify_port_error(e)),
                }
            }
            StorageOp::MonotonicUpsert(spec) => {
                match storage.monotonic_upsert(spec.clone()).await {
                    Ok(()) => {}
                    Err(e) => return PortOutcome::Err(classify_port_error(e)),
                }
            }
            StorageOp::Get(spec) => {
                match storage.get(spec.clone()).await {
                    Ok(Some(row)) => {
                        last_ok_bytes = rows_to_reply_bytes(&[row]);
                    }
                    Ok(None) => {
                        last_ok_bytes = Bytes::new();
                    }
                    Err(e) => return PortOutcome::Err(classify_port_error(e)),
                }
            }
            StorageOp::Scan(spec) => {
                match storage.scan(spec.clone()).await {
                    Ok(rows) => {
                        last_ok_bytes = rows_to_reply_bytes(&rows);
                    }
                    Err(e) => return PortOutcome::Err(classify_port_error(e)),
                }
            }
        }
    }

    PortOutcome::Ok(ReplyBytes(last_ok_bytes))
}

// ─── rows→bytes 序列化约定 ────────────────────────────────────────────────────

/// `&[Row]` → JSON array bytes（driver 约定序列化格式）。
///
/// ## Schema
///
/// ```json
/// [{"col1": "str", "col2": 42, "col3": 3.14, "col4": null}, ...]
/// ```
pub fn rows_to_reply_bytes(rows: &[helix_core::effect::Row]) -> Bytes {
    let json_rows: Vec<JsonValue> = rows
        .iter()
        .map(|row| {
            let obj: serde_json::Map<String, JsonValue> = row
                .iter()
                .map(|(k, v)| {
                    let jv = match v {
                        SqlValue::Text(s) => JsonValue::String(s.clone()),
                        SqlValue::Integer(i) => json!(i),
                        SqlValue::Real(f) => json!(f),
                        SqlValue::Null => JsonValue::Null,
                    };
                    (k.clone(), jv)
                })
                .collect();
            JsonValue::Object(obj)
        })
        .collect();

    serde_json::to_vec(&json_rows)
        .map(Bytes::from)
        .unwrap_or_default()
}

/// JSON bytes → `Vec<Row>`（`rows_to_reply_bytes` 的逆操作）。
///
/// number 类型：整型 → Integer；有小数点 → Real。
pub fn rows_from_reply_bytes(
    bytes: &Bytes,
) -> Result<Vec<helix_core::effect::Row>, String> {
    use helix_core::effect::{Row, SqlValue};

    let json: Vec<serde_json::Map<String, JsonValue>> =
        serde_json::from_slice(bytes).map_err(|e| e.to_string())?;

    let rows: Vec<Row> = json
        .into_iter()
        .map(|obj| {
            obj.into_iter()
                .map(|(k, v)| {
                    let sv = match v {
                        JsonValue::String(s) => SqlValue::Text(s),
                        JsonValue::Number(n) => {
                            if let Some(i) = n.as_i64() {
                                SqlValue::Integer(i)
                            } else {
                                SqlValue::Real(n.as_f64().unwrap_or(0.0))
                            }
                        }
                        JsonValue::Null => SqlValue::Null,
                        other => SqlValue::Text(other.to_string()),
                    };
                    (k, sv)
                })
                .collect()
        })
        .collect();

    Ok(rows)
}

// ─── 辅助函数 ────────────────────────────────────────────────────────────────

/// 从 StorageOp 列表提取第一个 op 的 table 名（用于分组并发）。
fn first_table_key(ops: &[StorageOp]) -> String {
    ops.first()
        .map(|op| match op {
            StorageOp::BatchUpsert(s) => s.table.to_string(),
            StorageOp::BatchUpdate(s) => s.table.to_string(),
            StorageOp::MonotonicUpsert(_) => "_helix_monotonic".to_string(),
            StorageOp::Get(s) => s.table.to_string(),
            StorageOp::Scan(s) => s.table.to_string(),
        })
        .unwrap_or_default()
}

/// helix_core::PortError（driver boundary）→ tick::PortError（模块可见）。
///
/// 模块用此类型区分 4xx/5xx/超时，实现差异化重试策略（MIN-3 落实）。
fn classify_port_error(e: PortError) -> CorePortError {
    match &e {
        PortError::Storage(_) => CorePortError::Storage(0),
        PortError::Transport(msg) if msg.starts_with("timeout") => CorePortError::Timeout,
        PortError::Transport(_) => CorePortError::Network,
        PortError::Http(msg) => {
            let code = msg
                .split_whitespace()
                .find_map(|w| w.parse::<u16>().ok())
                .unwrap_or(0);
            CorePortError::Http(code)
        }
        _ => CorePortError::Other(0),
    }
}

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
        out.push(if chunk.len() > 1 { TABLE[((n >> 6) & 63) as usize] as char } else { '=' });
        out.push(if chunk.len() > 2 { TABLE[(n & 63) as usize] as char } else { '=' });
    }
    out
}

// ─── 单元测试 ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use helix_core::effect::SqlValue;

    #[test]
    fn test_rows_roundtrip() {
        let rows = vec![
            vec![
                ("id".to_string(), SqlValue::Integer(1)),
                ("name".to_string(), SqlValue::Text("alice".to_string())),
                ("score".to_string(), SqlValue::Real(9.5)),
                ("extra".to_string(), SqlValue::Null),
            ],
        ];
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
    fn test_first_table_key_empty() {
        assert_eq!(first_table_key(&[]), "");
    }

    #[test]
    fn test_first_table_key_upsert() {
        use helix_core::effect::UpsertSpec;
        let ops = vec![StorageOp::BatchUpsert(UpsertSpec {
            table: "messages",
            rows: vec![],
            conflict_key: None,
        })];
        assert_eq!(first_table_key(&ops), "messages");
    }
}
