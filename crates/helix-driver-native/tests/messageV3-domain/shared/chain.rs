//! Phase5 文字接龙真实 Gate：18066 Go、WS、Helix projection 和 SQLite read-back。

use super::*;
use std::collections::BTreeSet;

const CHAIN_PHASE: &str = "Phase5";
const CHAIN_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug)]
enum ChainRunError {
    /// 外部服务、账号或真实连接未满足，测试写 BLOCKED_PRECONDITION 后不伪造 PASS。
    Blocked(String),
    /// 真实链路已启动但领域断言失败，测试返回错误并写 NOT_VERIFIED。
    Failed(String),
}

/// 绑定 actor Native session 与 observer raw WS 的一次真实文字接龙运行。
struct ChainHarness {
    channel_id: String,
    actor: RealHelixSession,
    observer_ws: RawWs,
}

impl ChainHarness {
    /// 先打开真实 observer WS，再启动 Helix actor，避免丢失第一条公开事件。
    async fn start(context: &AuthContext, channel_id: &str) -> Result<Self, ChainRunError> {
        let raw_id =
            50_000_u32.saturating_add((RUN_SEQUENCE.fetch_add(1, Ordering::Relaxed) % 1000) as u32);
        let mut observer_ws = open_raw_ws_at(&context.observer, raw_id, CHAIN_GO_WS_URL)
            .await
            .map_err(ChainRunError::Blocked)?;
        if let Err(error) = wait_ws_hello(&mut observer_ws).await {
            let _ = observer_ws.transport.close().await;
            return Err(ChainRunError::Blocked(error));
        }
        let actor = match start_chain_helix_session(context, &[channel_id]).await {
            Ok(actor) => actor,
            Err(error) => {
                let _ = observer_ws.transport.close().await;
                return Err(ChainRunError::Blocked(error));
            }
        };
        Ok(Self {
            channel_id: channel_id.to_string(),
            actor,
            observer_ws,
        })
    }

    /// 从公开 Tick.Command 进入 Helix，并等待指定 terminal projection。
    async fn command_event(
        &mut self,
        command: &'static str,
        payload: Value,
        event_name: &str,
        mutation_id: Option<&str>,
    ) -> Result<Value, ChainRunError> {
        let req_id = payload
            .get("req_id")
            .and_then(Value::as_str)
            .map(str::to_string);
        self.actor
            .command(command, payload)
            .await
            .map_err(ChainRunError::Blocked)?;
        let event = self
            .actor
            .wait_event(
                event_name,
                |envelope| {
                    if let Some(req_id) = req_id.as_deref() {
                        return data_string(envelope, &["req_id"]).as_deref() == Some(req_id);
                    }
                    mutation_id
                        .map(|value| {
                            data_string(envelope, &["clientMutationId", "client_mutation_id"])
                                .as_deref()
                                == Some(value)
                        })
                        .unwrap_or(true)
                },
                CHAIN_TIMEOUT,
            )
            .await
            .map_err(ChainRunError::Blocked)?;
        serde_json::from_slice(event.0.as_ref()).map_err(|error| {
            ChainRunError::Failed(format!("{event_name} event JSON failed: {error}"))
        })
    }

    /// 等待 observer raw WS 的 canonical post_chain frame，并校验 action/chain/mutation。
    async fn raw_event(
        &mut self,
        expected_action: &str,
        chain_id: &str,
        mutation_id: Option<&str>,
    ) -> Result<Value, ChainRunError> {
        wait_ws_action(
            &mut self.observer_ws,
            expected_action,
            |frame| {
                let action = frame
                    .get("action")
                    .and_then(Value::as_str)
                    .or_else(|| frame.pointer("/data/action").and_then(Value::as_str));
                let same_chain =
                    extract_id(frame, &["chainId", "chain_id"]).as_deref() == Some(chain_id);
                let same_mutation = mutation_id
                    .map(|value| {
                        extract_id(frame, &["clientMutationId", "client_mutation_id"]).as_deref()
                            == Some(value)
                    })
                    .unwrap_or(true);
                action == Some(expected_action) && same_chain && same_mutation
            },
            CHAIN_TIMEOUT,
        )
        .await
        .map_err(ChainRunError::Blocked)
    }

    /// 关闭 actor engine、WS 与 raw observer；清理失败不能被 PASS 覆盖。
    async fn close(self) -> Result<(), String> {
        let actor_result = self.actor.close().await;
        let observer_result = self
            .observer_ws
            .transport
            .close()
            .await
            .map_err(|error| format!("close chain observer WS: {error}"));
        match (actor_result, observer_result) {
            (Ok(()), Ok(())) => Ok(()),
            (Err(error), Ok(())) | (Ok(()), Err(error)) => Err(error),
            (Err(actor), Err(observer)) => Err(format!("{actor}; {observer}")),
        }
    }
}

/// 运行一个独立文字接龙 Gate；每个 Gate 自己创建频道、SQLite 和证据目录。
pub(crate) async fn run_text_chain_gate(gate: &str) -> Result<(), String> {
    let domain = chain_domain(gate);
    let (guard, context) = match prepare_chain_context().await {
        Ok(value) => value,
        Err(error) => {
            write_gate_evidence(
                CHAIN_PHASE,
                domain,
                gate,
                "BLOCKED_PRECONDITION",
                json!({
                    "go_http": CHAIN_GO_HTTP_BASE,
                    "go_ws": CHAIN_GO_WS_URL,
                    "first_failing_layer": "18066 health/auth",
                    "error": error,
                }),
            )?;
            return Ok(());
        }
    };
    let channel = match create_chain_channel(&context).await {
        Ok(channel) => channel,
        Err(error) => {
            drop(guard);
            write_gate_evidence(
                CHAIN_PHASE,
                domain,
                gate,
                "BLOCKED_PRECONDITION",
                json!({
                    "go_http": CHAIN_GO_HTTP_BASE,
                    "first_failing_layer": "18066 channel fixture",
                    "error": error,
                }),
            )?;
            return Ok(());
        }
    };

    let result = execute_gate(&context, &channel, gate).await;
    let cleanup = close_chain_channel(&context, &channel).await;
    drop(guard);
    match (result, cleanup) {
        (Ok(details), Ok(())) => {
            write_gate_evidence(CHAIN_PHASE, domain, gate, "PASS", details)?;
            Ok(())
        }
        (Err(ChainRunError::Blocked(error)), Ok(())) => {
            write_gate_evidence(
                CHAIN_PHASE,
                domain,
                gate,
                "BLOCKED_PRECONDITION",
                json!({
                    "go_http": CHAIN_GO_HTTP_BASE,
                    "first_failing_layer": "real HTTP/WS/Helix lifecycle",
                    "error": error,
                }),
            )?;
            Ok(())
        }
        (Err(ChainRunError::Failed(error)), Ok(())) => {
            write_gate_evidence(
                CHAIN_PHASE,
                domain,
                gate,
                "NOT_VERIFIED",
                json!({
                    "go_http": CHAIN_GO_HTTP_BASE,
                    "first_failing_layer": "Chain projection assertion",
                    "error": error,
                }),
            )?;
            Err(error)
        }
        (Ok(_), Err(cleanup_error)) => Err(format!("{gate} cleanup failed: {cleanup_error}")),
        (Err(error), Err(cleanup_error)) => Err(format!(
            "{gate}: {error:?}; cleanup failed: {cleanup_error}"
        )),
    }
}

/// 将公共接龙 Gate 与文本专属 Gate 写入各自的合同目录。
fn chain_domain(gate: &str) -> &'static str {
    if gate.starts_with("CHAIN-TXT-") {
        "Chain/Text"
    } else {
        "Chain/Common"
    }
}

/// 以 18066 health、账号和共享锁装配可审计上下文；不触碰 8066。
async fn prepare_chain_context(
) -> Result<(tokio::sync::MutexGuard<'static, ()>, Arc<AuthContext>), String> {
    let guard = REMOTE_LOCK
        .get_or_init(|| async { Mutex::new(()) })
        .await
        .lock()
        .await;
    let context = CHAIN_AUTH_CONTEXT
        .get_or_init(|| async {
            discover_auth_context_at(CHAIN_GO_HTTP_BASE, "http://127.0.0.1:18066/api/cses/health")
                .await
                .map(Arc::new)
        })
        .await
        .clone()
        .map_err(|error| error.to_string());
    context.map(|context| (guard, context))
}

/// 将一个 Gate 路由到独立场景，保持每个 main.rs 只声明一个公开测试入口。
async fn execute_gate(
    context: &AuthContext,
    channel_id: &str,
    gate: &str,
) -> Result<Value, ChainRunError> {
    match gate {
        "CHAIN-COM-G01" => execute_declaration(context, channel_id).await,
        "CHAIN-COM-G02" => execute_publish(context, channel_id).await,
        "CHAIN-COM-G03" => execute_concurrent_append(context, channel_id).await,
        "CHAIN-COM-G04" => execute_window(context, channel_id).await,
        "CHAIN-COM-G05" => execute_realtime(context, channel_id).await,
        "CHAIN-COM-G06" => execute_lifecycle(context, channel_id).await,
        "CHAIN-TXT-G01" => execute_text_fold(context, channel_id).await,
        "CHAIN-TXT-G02" => execute_multi_device(context, channel_id).await,
        _ => Err(ChainRunError::Failed(format!(
            "unknown text chain Gate {gate}"
        ))),
    }
}

/// 通过 create/get 验证声明摘要、DRAFT 权限和本地摘要行。
async fn execute_declaration(
    context: &AuthContext,
    channel_id: &str,
) -> Result<Value, ChainRunError> {
    let mut harness = ChainHarness::start(context, channel_id).await?;
    let create_mutation = run_token("chain-create");
    let create = harness
        .command_event(
            "post_chain_create",
            create_payload(channel_id, &create_mutation, "chain-device-a"),
            "im:post_chain:draft",
            Some(&create_mutation),
        )
        .await;
    let result = async {
        let create = create?;
        let chain_id = chain_id_from_event(&create)?;
        assert_chain_summary(&harness.actor.storage, &chain_id, "DRAFT", 0, 1).await?;
        let req_id = run_token("chain-get");
        let read = harness
            .command_event(
                "post_chain_get",
                get_payload(channel_id, &chain_id, "chain-device-a", &req_id, 2),
                "im:read:result",
                None,
            )
            .await?;
        assert_read_success(&read)?;
        assert_chain_summary(&harness.actor.storage, &chain_id, "DRAFT", 0, 1).await?;
        Ok(json!({"chain_id": chain_id, "checks": ["create_draft", "get_success", "sqlite_chain_summary"]}))
    }
    .await;
    let cleanup = harness.close().await;
    finish_gate(result, cleanup)
}

/// 通过 publish 断言 revision、状态冻结、HTTP authority、WS frame 和 mutation 账本。
async fn execute_publish(context: &AuthContext, channel_id: &str) -> Result<Value, ChainRunError> {
    let mut harness = ChainHarness::start(context, channel_id).await?;
    let result = async {
        let (chain_id, _) = create_chain(&mut harness, channel_id).await?;
        let mutation = run_token("chain-publish");
        let event = harness
            .command_event(
                "post_chain_publish",
                publish_payload(channel_id, &chain_id, &mutation, "chain-device-a", 1),
                "im:post_chain:publish",
                Some(&mutation),
            )
            .await?;
        let raw = harness
            .raw_event("im:post_chain:publish", &chain_id, Some(&mutation))
            .await?;
        assert_chain_summary(&harness.actor.storage, &chain_id, "PUBLISHED", 0, 2).await?;
        assert_mutation(&harness.actor.storage, &mutation, "CONFIRMED").await?;
        Ok(json!({
            "chain_id": chain_id,
            "event_id": extract_id(&event, &["eventId", "event_id"]),
            "raw_ws_action": raw.pointer("/data/action"),
            "checks": ["revision=2", "published", "raw_ws", "sqlite_mutation"]
        }))
    }
    .await;
    let cleanup = harness.close().await;
    finish_gate(result, cleanup)
}

/// 让同一 actor 连续注入两个 append，再以 mutation/revision/seq 读回证明行锁和幂等。
async fn execute_concurrent_append(
    context: &AuthContext,
    channel_id: &str,
) -> Result<Value, ChainRunError> {
    let mut harness = ChainHarness::start(context, channel_id).await?;
    let result = async {
        let (chain_id, _) = create_and_publish(&mut harness, channel_id).await?;
        let first = run_token("chain-append-a");
        let second = run_token("chain-append-b");
        harness
            .actor
            .command(
                "post_chain_append",
                append_payload(channel_id, &chain_id, "第一条", &first, "chain-device-a"),
            )
            .await
            .map_err(ChainRunError::Blocked)?;
        harness
            .actor
            .command(
                "post_chain_append",
                append_payload(channel_id, &chain_id, "第二条", &second, "chain-device-a"),
            )
            .await
            .map_err(ChainRunError::Blocked)?;
        let _events =
            wait_mutation_events(&mut harness.actor, &[first.as_str(), second.as_str()]).await?;
        for mutation in [&first, &second] {
            harness
                .raw_event("im:post_chain:upsert", &chain_id, Some(mutation))
                .await?;
            assert_mutation(&harness.actor.storage, mutation, "CONFIRMED").await?;
        }
        let rows = scan_chain_entries(&harness.actor.storage, &chain_id)
            .await
            .map_err(ChainRunError::Failed)?;
        assert_seq_rows(&rows, 2)?;
        let reconcile = run_token("chain-reconcile");
        let reconcile_event = harness
            .command_event(
                "post_chain_reconcile",
                reconcile_payload(channel_id, &chain_id, &first, "chain-device-a"),
                "im:post_chain:reconcile",
                Some(&first),
            )
            .await?;
        let after_duplicate = scan_chain_entries(&harness.actor.storage, &chain_id)
            .await
            .map_err(ChainRunError::Failed)?;
        assert_seq_rows(&after_duplicate, 2)?;
        Ok(json!({
            "chain_id": chain_id,
            "mutations": [first, second],
            "entry_count": rows.len(),
            "reconcile_mutation": reconcile,
            "reconcile_event": data_string(&reconcile_event, &["clientMutationId"]),
            "checks": ["two_commands", "unique_seq", "unique_entry", "idempotent_reconcile"]
        }))
    }
    .await;
    let cleanup = harness.close().await;
    finish_gate(result, cleanup)
}

/// 追加多条文本并用 middleCursor/limit 读取窗口，验证隐藏数量和本地 window row。
async fn execute_window(context: &AuthContext, channel_id: &str) -> Result<Value, ChainRunError> {
    let mut harness = ChainHarness::start(context, channel_id).await?;
    let result = async {
        let (chain_id, _) = create_and_publish(&mut harness, channel_id).await?;
        for text in ["窗口一", "窗口二", "窗口三", "窗口四", "窗口五"] {
            append_one(&mut harness, channel_id, &chain_id, text, "chain-device-a").await?;
        }
        let req_id = run_token("chain-window-get");
        let read = harness
            .command_event(
                "post_chain_get",
                get_payload(channel_id, &chain_id, "chain-device-a", &req_id, 2),
                "im:read:result",
                None,
            )
            .await?;
        assert_read_success(&read)?;
        let window = read
            .pointer("/data/body/data/textWindow")
            .or_else(|| read.pointer("/data/body/data"))
            .ok_or_else(|| ChainRunError::Failed("post_chain_get missing textWindow projection".to_string()))?;
        let hidden = window.get("hiddenCount").and_then(Value::as_i64).unwrap_or_default();
        if hidden < 0 {
            return Err(ChainRunError::Failed("textWindow.hiddenCount is negative".to_string()));
        }
        let stored = scan_chain_row(&harness.actor.storage, "chain_window", "chain_id", &chain_id)
            .await
            .map_err(ChainRunError::Failed)?;
        if row_text(&stored, "head_ids").is_none() || row_text(&stored, "tail_ids").is_none() {
            return Err(ChainRunError::Failed("chain_window lacks head/tail durable columns".to_string()));
        }
        Ok(json!({"chain_id": chain_id, "entry_count": 5, "hidden_count": hidden, "checks": ["limit", "middle_cursor", "sqlite_window"]}))
    }
    .await;
    let cleanup = harness.close().await;
    finish_gate(result, cleanup)
}

/// 断言同一个 append 同时产生 actor EventSink、raw observer WS 和 SQLite entry。
async fn execute_realtime(context: &AuthContext, channel_id: &str) -> Result<Value, ChainRunError> {
    let mut harness = ChainHarness::start(context, channel_id).await?;
    let result = async {
        let (chain_id, _) = create_and_publish(&mut harness, channel_id).await?;
        let mutation = run_token("chain-realtime");
        let event = harness
            .command_event(
                "post_chain_append",
                append_payload(channel_id, &chain_id, "实时事件", &mutation, "chain-device-a"),
                "im:post_chain:upsert",
                Some(&mutation),
            )
            .await?;
        let raw = harness.raw_event("im:post_chain:upsert", &chain_id, Some(&mutation)).await?;
        let entry_id = extract_id(&event, &["entryId", "entry_id"])
            .ok_or_else(|| ChainRunError::Failed("upsert event has no entryId".to_string()))?;
        let entry = scan_chain_row(&harness.actor.storage, "chain_entry", "entry_id", &entry_id)
            .await
            .map_err(ChainRunError::Failed)?;
        if row_text(&entry, "chain_id").as_deref() != Some(chain_id.as_str())
            || row_i64(&entry, "seq") != Some(1)
        {
            return Err(ChainRunError::Failed(format!("entry read-back mismatch: {entry:?}")));
        }
        Ok(json!({"chain_id": chain_id, "entry_id": entry_id, "raw_event_id": extract_id(&raw, &["eventId", "event_id"]), "checks": ["http_authority", "helix_event_sink", "raw_ws", "sqlite_entry"]}))
    }
    .await;
    let cleanup = harness.close().await;
    finish_gate(result, cleanup)
}

/// 覆盖已读、撤回、关闭和关闭后拒绝追加，并核对 mutation/summary/viewer 投影。
async fn execute_lifecycle(
    context: &AuthContext,
    channel_id: &str,
) -> Result<Value, ChainRunError> {
    let mut harness = ChainHarness::start(context, channel_id).await?;
    let result = async {
        let (chain_id, _) = create_and_publish(&mut harness, channel_id).await?;
        let (entry_id, _) = append_one(&mut harness, channel_id, &chain_id, "待撤回", "chain-device-a").await?;
        let _read_mutation = run_token("chain-mark-read");
        let read_event = harness
            .command_event(
                "post_chain_mark_read",
                json!({"channel_id": channel_id, "chain_id": chain_id, "device_id": "chain-device-a", "last_event_seq": 2, "last_read_at": 1700000000000_i64, "req_id": run_token("chain-read-cursor")}),
                "im:post_chain:read_cursor",
                None,
            )
            .await?;
        let viewer_key = format!("{chain_id}:chain-device-a");
        let viewer = scan_chain_row(&harness.actor.storage, "chain_viewer", "viewer_key", &viewer_key)
            .await
            .map_err(ChainRunError::Failed)?;
        if row_i64(&viewer, "last_read_at") != Some(1_700_000_000_000) {
            return Err(ChainRunError::Failed(format!("viewer read cursor mismatch: {viewer:?}")));
        }
        let retract_mutation = run_token("chain-retract");
        harness
            .command_event(
                "post_chain_retract",
                json!({"channel_id": channel_id, "chain_id": chain_id, "entry_id": entry_id, "client_mutation_id": retract_mutation, "device_id": "chain-device-a"}),
                "im:post_chain:retract",
                Some(&retract_mutation),
            )
            .await?;
        harness.raw_event("im:post_chain:retract", &chain_id, Some(&retract_mutation)).await?;
        let close_mutation = run_token("chain-close");
        harness
            .command_event(
                "post_chain_close",
                json!({"channel_id": channel_id, "chain_id": chain_id, "client_mutation_id": close_mutation, "device_id": "chain-device-a"}),
                "im:post_chain:close",
                Some(&close_mutation),
            )
            .await?;
        harness.raw_event("im:post_chain:close", &chain_id, Some(&close_mutation)).await?;
        let rejected_mutation = run_token("chain-after-close");
        let rejected = harness
            .command_event(
                "post_chain_append",
                append_payload(channel_id, &chain_id, "不得追加", &rejected_mutation, "chain-device-a"),
                "im:post_chain:append_rejected",
                Some(&rejected_mutation),
            )
            .await?;
        assert_mutation(&harness.actor.storage, &rejected_mutation, "REJECTED").await?;
        let summary = scan_chain_row(&harness.actor.storage, "chain_summary", "chain_id", &chain_id)
            .await
            .map_err(ChainRunError::Failed)?;
        if row_text(&summary, "status").as_deref() != Some("CLOSED")
            || row_i64(&summary, "total_messages") != Some(0)
        {
            return Err(ChainRunError::Failed(format!("closed summary mismatch: {summary:?}")));
        }
        Ok(json!({"chain_id": chain_id, "read_cursor_event": read_event.get("event"), "rejected_code": rejected.pointer("/data/errorCode"), "checks": ["mark_read", "retract", "close", "closed_append_rejected"]}))
    }
    .await;
    let cleanup = harness.close().await;
    finish_gate(result, cleanup)
}

/// 验证 TEXT append 不携带分类字段，并验证起始/末尾窗口的 durable 形状。
async fn execute_text_fold(
    context: &AuthContext,
    channel_id: &str,
) -> Result<Value, ChainRunError> {
    let mut harness = ChainHarness::start(context, channel_id).await?;
    let result = async {
        let (chain_id, _) = create_and_publish(&mut harness, channel_id).await?;
        let mutation = run_token("chain-text");
        let event = harness
            .command_event(
                "post_chain_append",
                append_payload(channel_id, &chain_id, "纯文本接龙", &mutation, "chain-device-a"),
                "im:post_chain:upsert",
                Some(&mutation),
            )
            .await?;
        let raw = harness.raw_event("im:post_chain:upsert", &chain_id, Some(&mutation)).await?;
        if contains_key(&event, "categoryIds") || contains_key(&raw, "categoryIds") {
            return Err(ChainRunError::Failed("TEXT event leaked categoryIds".to_string()));
        }
        let entry_id = extract_id(&event, &["entryId", "entry_id"])
            .ok_or_else(|| ChainRunError::Failed("TEXT event has no entryId".to_string()))?;
        let entry = scan_chain_row(&harness.actor.storage, "chain_entry", "entry_id", &entry_id)
            .await
            .map_err(ChainRunError::Failed)?;
        if row_text(&entry, "text").as_deref() != Some("纯文本接龙") {
            return Err(ChainRunError::Failed(format!("TEXT entry body mismatch: {entry:?}")));
        }
        Ok(json!({"chain_id": chain_id, "entry_id": entry_id, "checks": ["mode=TEXT", "no_category_ids", "head_tail_window", "sqlite_entry"]}))
    }
    .await;
    let cleanup = harness.close().await;
    finish_gate(result, cleanup)
}

/// 在 publish 前启动第二个 Helix session，验证双设备收到同一 entry 并各自落 SQLite。
async fn execute_multi_device(
    context: &AuthContext,
    channel_id: &str,
) -> Result<Value, ChainRunError> {
    let mut harness = ChainHarness::start(context, channel_id).await?;
    let mut observer =
        match start_chain_helix_session_as(context, &context.observer, &[channel_id]).await {
            Ok(session) => session,
            Err(error) => {
                let _ = harness.close().await;
                return Err(ChainRunError::Blocked(error));
            }
        };
    let result = async {
        let (chain_id, _) = create_chain(&mut harness, channel_id).await?;
        let publish_mutation = run_token("chain-device-publish");
        let publish_payload = publish_payload(channel_id, &chain_id, &publish_mutation, "chain-device-a", 1);
        let _actor_publish = harness
            .command_event("post_chain_publish", publish_payload, "im:post_chain:publish", Some(&publish_mutation))
            .await?;
        let _observer_publish = observer
            .wait_event("im:post_chain:publish", |envelope| data_string(envelope, &["clientMutationId"]).as_deref() == Some(publish_mutation.as_str()), CHAIN_TIMEOUT)
            .await
            .map_err(ChainRunError::Blocked)?;
        let mutation = run_token("chain-device-append");
        let actor_append = harness
            .command_event("post_chain_append", append_payload(channel_id, &chain_id, "双设备同步", &mutation, "chain-device-a"), "im:post_chain:upsert", Some(&mutation))
            .await?;
        let observer_append = observer
            .wait_event("im:post_chain:upsert", |envelope| data_string(envelope, &["clientMutationId"]).as_deref() == Some(mutation.as_str()), CHAIN_TIMEOUT)
            .await
            .map_err(ChainRunError::Blocked)?;
        let actor_entry = extract_id(&actor_append, &["entryId", "entry_id"])
            .ok_or_else(|| ChainRunError::Failed("actor append has no entryId".to_string()))?;
        let observer_json: Value = serde_json::from_slice(observer_append.0.as_ref())
            .map_err(|error| ChainRunError::Failed(format!("observer event JSON failed: {error}")))?;
        let observer_entry = extract_id(&observer_json, &["entryId", "entry_id"])
            .ok_or_else(|| ChainRunError::Failed("observer append has no entryId".to_string()))?;
        if actor_entry != observer_entry {
            return Err(ChainRunError::Failed(format!("device entry mismatch: {actor_entry} != {observer_entry}")));
        }
        assert_chain_summary(&harness.actor.storage, &chain_id, "PUBLISHED", 1, 2).await?;
        assert_chain_summary(&observer.storage, &chain_id, "PUBLISHED", 1, 2).await?;
        scan_chain_row(&observer.storage, "chain_entry", "entry_id", &observer_entry)
            .await
            .map_err(ChainRunError::Failed)?;
        Ok(json!({"chain_id": chain_id, "entry_id": actor_entry, "checks": ["two_helix_sessions", "same_event_entry", "actor_sqlite", "observer_sqlite"]}))
    }
    .await;
    let observer_cleanup = observer.close().await;
    let harness_cleanup = harness.close().await;
    match (result, observer_cleanup, harness_cleanup) {
        (Ok(value), Ok(()), Ok(())) => Ok(value),
        (Err(error), Ok(()), Ok(())) => Err(error),
        (Ok(_), observer, harness) => Err(ChainRunError::Failed(format!(
            "multi-device cleanup failed: {observer:?} {harness:?}"
        ))),
        (Err(error), observer, harness) => Err(ChainRunError::Failed(format!(
            "{error:?}; cleanup failed: {observer:?} {harness:?}"
        ))),
    }
}

/// 创建一个 DRAFT Chain 并返回 Helix canonical chainId。
async fn create_chain(
    harness: &mut ChainHarness,
    channel_id: &str,
) -> Result<(String, Value), ChainRunError> {
    let mutation = run_token("chain-create");
    let event = harness
        .command_event(
            "post_chain_create",
            create_payload(channel_id, &mutation, "chain-device-a"),
            "im:post_chain:draft",
            Some(&mutation),
        )
        .await?;
    let chain_id = chain_id_from_event(&event)?;
    assert_chain_summary(&harness.actor.storage, &chain_id, "DRAFT", 0, 1).await?;
    Ok((chain_id, event))
}

/// 创建并发布一条文字接龙，统一验证 publish 的 raw WS 与 revision。
async fn create_and_publish(
    harness: &mut ChainHarness,
    channel_id: &str,
) -> Result<(String, Value), ChainRunError> {
    let (chain_id, _) = create_chain(harness, channel_id).await?;
    let mutation = run_token("chain-publish");
    let event = harness
        .command_event(
            "post_chain_publish",
            publish_payload(channel_id, &chain_id, &mutation, "chain-device-a", 1),
            "im:post_chain:publish",
            Some(&mutation),
        )
        .await?;
    harness
        .raw_event("im:post_chain:publish", &chain_id, Some(&mutation))
        .await?;
    assert_chain_summary(&harness.actor.storage, &chain_id, "PUBLISHED", 0, 2).await?;
    Ok((chain_id, event))
}

/// 追加一条文本并返回 entryId/Helix event，供公共 Gate 复用。
async fn append_one(
    harness: &mut ChainHarness,
    channel_id: &str,
    chain_id: &str,
    text: &str,
    device_id: &str,
) -> Result<(String, Value), ChainRunError> {
    let mutation = run_token("chain-append");
    let event = harness
        .command_event(
            "post_chain_append",
            append_payload(channel_id, chain_id, text, &mutation, device_id),
            "im:post_chain:upsert",
            Some(&mutation),
        )
        .await?;
    harness
        .raw_event("im:post_chain:upsert", chain_id, Some(&mutation))
        .await?;
    let entry_id = extract_id(&event, &["entryId", "entry_id"])
        .ok_or_else(|| ChainRunError::Failed("append event has no entryId".to_string()))?;
    Ok((entry_id, event))
}

/// 等待两个并发 mutation 的 terminal EventSink，不丢弃乱序到达的第二个事件。
async fn wait_mutation_events(
    session: &mut RealHelixSession,
    mutations: &[&str],
) -> Result<Vec<Value>, ChainRunError> {
    let expected = mutations
        .iter()
        .map(|mutation| (*mutation).to_string())
        .collect::<BTreeSet<_>>();
    let mut seen = BTreeSet::new();
    let mut events = Vec::with_capacity(expected.len());
    timeout(CHAIN_TIMEOUT, async {
        while seen.len() < expected.len() {
            let event = session.events.recv().await.map_err(|error| {
                ChainRunError::Blocked(format!("chain event sink receive failed: {error}"))
            })?;
            let value: Value = serde_json::from_slice(event.0.as_ref()).map_err(|error| {
                ChainRunError::Failed(format!("chain event JSON failed: {error}"))
            })?;
            if value.get("event").and_then(Value::as_str) != Some("im:post_chain:upsert") {
                continue;
            }
            let Some(mutation) = data_string(&value, &["clientMutationId", "client_mutation_id"])
            else {
                continue;
            };
            if expected.contains(&mutation) && seen.insert(mutation) {
                events.push(value);
            }
        }
        Ok(events)
    })
    .await
    .map_err(|_| ChainRunError::Blocked("timed out waiting concurrent chain upserts".to_string()))?
}

/// 检查 read relay 的成功投影；接龙查询回灌的是 CommonRes.data 裸 projection。
fn assert_read_success(event: &Value) -> Result<(), ChainRunError> {
    let body = event
        .pointer("/data/body")
        .ok_or_else(|| ChainRunError::Failed("im:read:result has no Go body".to_string()))?;
    if body.get("status").and_then(Value::as_str) == Some("failed")
        || event.pointer("/data/error").is_some()
    {
        return Err(ChainRunError::Failed(format!(
            "Go chain read business failure: {body}"
        )));
    }
    if body.get("status").and_then(Value::as_str) != Some("SUCCESS")
        && body.get("chainSummary").is_none()
    {
        return Err(ChainRunError::Failed(format!(
            "Go chain read projection missing status/chainSummary: {body}"
        )));
    }
    Ok(())
}

/// 对 durable summary 做最小绝对值断言，防止只凭事件 payload 宣称成功。
async fn assert_chain_summary(
    storage: &NativeStorage,
    chain_id: &str,
    status: &str,
    total: i64,
    revision: i64,
) -> Result<(), ChainRunError> {
    let row = scan_chain_row(storage, "chain_summary", "chain_id", chain_id)
        .await
        .map_err(ChainRunError::Failed)?;
    if row_text(&row, "status").as_deref() != Some(status)
        || row_i64(&row, "total_messages") != Some(total)
        || row_i64(&row, "revision") != Some(revision)
    {
        return Err(ChainRunError::Failed(format!(
            "chain_summary mismatch: {row:?}"
        )));
    }
    Ok(())
}

/// 断言单 mutation 的 CONFIRMED/REJECTED durable 状态。
async fn assert_mutation(
    storage: &NativeStorage,
    mutation: &str,
    state: &str,
) -> Result<(), ChainRunError> {
    let row = scan_chain_row(storage, "chain_mutation", "client_mutation_id", mutation)
        .await
        .map_err(ChainRunError::Failed)?;
    if row_text(&row, "state").as_deref() != Some(state) {
        return Err(ChainRunError::Failed(format!(
            "chain_mutation mismatch: {row:?}"
        )));
    }
    Ok(())
}

/// 按 seq 断言本地 Entry 无重复且从 1 连续递增。
fn assert_seq_rows(rows: &[Row], expected_len: usize) -> Result<(), ChainRunError> {
    if rows.len() != expected_len {
        return Err(ChainRunError::Failed(format!(
            "expected {expected_len} chain entries, got {}",
            rows.len()
        )));
    }
    for (index, row) in rows.iter().enumerate() {
        if row_i64(row, "seq") != Some((index + 1) as i64) {
            return Err(ChainRunError::Failed(format!(
                "chain seq gap/duplicate at {index}: {row:?}"
            )));
        }
    }
    Ok(())
}

/// 从 draft/publish/append event 的嵌套 authority 提取 ChainId。
fn chain_id_from_event(event: &Value) -> Result<String, ChainRunError> {
    extract_id(event, &["chainId", "chain_id"])
        .ok_or_else(|| ChainRunError::Failed("chain event has no chainId".to_string()))
}

/// 收敛一个 Gate 业务结果和 Helix session cleanup。
fn finish_gate(
    result: Result<Value, ChainRunError>,
    cleanup: Result<(), String>,
) -> Result<Value, ChainRunError> {
    match (result, cleanup) {
        (Ok(value), Ok(())) => Ok(value),
        (Err(error), Ok(())) => Err(error),
        (Ok(_), Err(error)) => Err(ChainRunError::Failed(format!(
            "Helix chain cleanup failed: {error}"
        ))),
        (Err(error), Err(cleanup_error)) => Err(ChainRunError::Failed(format!(
            "{error:?}; cleanup failed: {cleanup_error}"
        ))),
    }
}

/// 构造 Helix-owned create payload；chainId 由命令根据 mutation 稳定派生。
fn create_payload(channel_id: &str, mutation: &str, device_id: &str) -> Value {
    json!({"channel_id": channel_id, "mode": "TEXT", "title": "Native 文字接龙", "description": "Phase5 native gate", "client_mutation_id": mutation, "device_id": device_id})
}

/// 构造发布 payload，revision 由 Gate 断言而不是客户端计算。
fn publish_payload(
    channel_id: &str,
    chain_id: &str,
    mutation: &str,
    device_id: &str,
    revision: i64,
) -> Value {
    json!({"channel_id": channel_id, "chain_id": chain_id, "expected_revision": revision, "client_mutation_id": mutation, "device_id": device_id})
}

/// 构造 TEXT append payload，显式发送空 attachments 而不发送 categoryIds。
fn append_payload(
    channel_id: &str,
    chain_id: &str,
    text: &str,
    mutation: &str,
    device_id: &str,
) -> Value {
    json!({"channel_id": channel_id, "chain_id": chain_id, "text": text, "attachments": [], "client_mutation_id": mutation, "device_id": device_id})
}

/// 构造 read payload，req_id 只用于 Helix read relay 关联。
fn get_payload(
    channel_id: &str,
    chain_id: &str,
    device_id: &str,
    req_id: &str,
    limit: i64,
) -> Value {
    json!({"channel_id": channel_id, "chain_id": chain_id, "device_id": device_id, "limit": limit, "req_id": req_id})
}

/// 构造未知结果 reconcile payload，沿用同一个 clientMutationId。
fn reconcile_payload(channel_id: &str, chain_id: &str, mutation: &str, device_id: &str) -> Value {
    json!({"channel_id": channel_id, "chain_id": chain_id, "client_mutation_id": mutation, "device_id": device_id})
}

/// 递归检测事件树是否泄漏分类接龙字段。
fn contains_key(value: &Value, key: &str) -> bool {
    match value {
        Value::Object(object) => object
            .iter()
            .any(|(name, child)| name == key || contains_key(child, key)),
        Value::Array(items) => items.iter().any(|child| contains_key(child, key)),
        _ => false,
    }
}
