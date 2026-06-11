//! engine_loop — tokio 事件泵
//!
//! ## tokio::spawn 在此，不在 helix-core
//!
//! helix-core 保持零运行时依赖；所有异步协调（WS recv、HTTP 响应、timer fired、
//! AppCommand）在此 tokio mpsc 统一入站。
//!
//! ## 架构
//!
//! ```text
//! +- tick_tx mpsc::channel ------------------------------------------+
//! |  WS reads (per transport) -> Tick::Inbound / Connected / Disconnected |
//! |  timer spawn             -> Tick::Timer                          |
//! |  HTTP spawn              -> Tick::PortReply                      |
//! |  AppCommand (external)   -> Tick::Command                        |
//! +------------------------------------------------------------------+
//!             v tick_rx.recv()
//!     ExecutionShell::step(tick, now_ms)
//!             v &[Effect] (borrows shell.scratch, valid until next step)
//!     consume_effects() -> collect to OwnedEffect (frees borrow)
//!     dispatch_owned_effects() -> dispatch by type
//! ```
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
use tokio::sync::{mpsc, Semaphore};

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

/// 跨组并发存储操作的最大并发度
const STORAGE_CONCURRENCY: usize = 4;

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
    let sem = Arc::new(Semaphore::new(STORAGE_CONCURRENCY));

    // 初始化：触发 on_start
    // 立即 consume → 脱离 &shell.scratch 借用，再进行 async 兑现
    let initial_effects = match shell.start() {
        Ok(effects) => consume_effects(effects),
        Err(e) => {
            tracing::error!("helix engine start failed: {}", e);
            return;
        }
    };
    tracing::info!("helix engine started, {} initial effects", initial_effects.len());

    dispatch_owned_effects(
        initial_effects,
        &tick_tx,
        &storage,
        &http,
        &event_sink,
        &mut timer_registry,
        &sem,
    ).await;

    // 事件泵主循环
    while let Some(tick) = tick_rx.recv().await {
        let now_ms = clock.now_ms();
        let owned = match shell.step(tick, now_ms) {
            Ok(effects) => consume_effects(effects), // 立即脱离借用
            Err(e) => {
                tracing::error!("engine step error: {}", e);
                // 非致命：记录继续，避免单个坏帧击沉整个 engine
                continue;
            }
        };

        if !owned.is_empty() {
            dispatch_owned_effects(
                owned,
                &tick_tx,
                &storage,
                &http,
                &event_sink,
                &mut timer_registry,
                &sem,
            ).await;
        }
    }

    tracing::info!("helix engine loop exited (tick channel closed)");
}

/// 把一批 OwnedEffect 分流兑现。
async fn dispatch_owned_effects(
    effects: Vec<OwnedEffect>,
    tick_tx: &mpsc::Sender<Tick>,
    storage: &Arc<NativeStorage>,
    http: &Arc<NativeHttp>,
    event_sink: &Arc<NativeEventSink>,
    timer_registry: &mut TimerRegistry,
    sem: &Arc<Semaphore>,
) {
    // 按 table 分组的 Persist 任务列表：table_name → vec[(corr?, ops)]
    // 同 table 内的操作保证串行（避免写顺序乱序 + SQLite 锁争用）
    let mut persist_groups: HashMap<String, Vec<(Option<Correlation>, Vec<StorageOp>)>> =
        HashMap::new();

    for effect in effects {
        match effect {
            // ── Persist（带 corr，兑现后回报）──────────────────────────────
            OwnedEffect::Persist { corr, ops } => {
                let key = first_table_key(&ops);
                persist_groups.entry(key).or_default().push((Some(corr), ops));
            }

            // ── PersistFire（fire-and-forget，不产出 PortReply）────────────
            OwnedEffect::PersistFire { ops } => {
                let key = first_table_key(&ops);
                persist_groups.entry(key).or_default().push((None, ops));
            }

            // ── Http（并发，带 corr 回报）──────────────────────────────────
            OwnedEffect::Http { corr, req } => {
                let http_clone = Arc::clone(http);
                let tx_clone = tick_tx.clone();
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
                    tx_clone.send(Tick::PortReply { corr, outcome }).await.ok();
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
                timer_registry.schedule(id, after_ms, tick_tx.clone());
            }

            // ── CancelTimer（幂等）────────────────────────────────────────
            OwnedEffect::CancelTimer { id } => {
                timer_registry.cancel(id);
            }
        }
    }

    // 并发执行各组 Persist 任务（跨组并发，组内顺序）
    if !persist_groups.is_empty() {
        let mut handles = Vec::new();
        for (_table, group) in persist_groups {
            let storage_clone = Arc::clone(storage);
            let tx_clone = tick_tx.clone();
            let sem_clone = Arc::clone(sem);

            let handle = tokio::spawn(async move {
                let _permit = sem_clone.acquire().await.ok();
                for (corr_opt, ops) in group {
                    let outcome = execute_storage_ops(&storage_clone, &ops).await;
                    if let Some(corr) = corr_opt {
                        tx_clone
                            .send(Tick::PortReply { corr, outcome })
                            .await
                            .ok();
                    }
                    // PersistFire (corr_opt = None) → 不回报，fire-and-forget
                }
            });
            handles.push(handle);
        }
        // 等待所有组完成（保证本次 step 的所有 Persist 兑现后再处理下一个 Tick）
        for h in handles {
            h.await.ok();
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
                        // rows→bytes 约定：JSON array，详见文件顶注释
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
