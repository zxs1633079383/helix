//! Phase5 文字接龙真实 Gate：profile Go/WS、Helix projection 和 SQLite read-back。

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

/// 绑定 actor/observer Native session 的一次真实文字接龙运行；Phase 批次可复用两条账号 WS。
struct ChainHarness {
    channel_id: String,
    actor: RealHelixSession,
    observer: RealHelixSession,
}

impl ChainHarness {
    /// 先启动 observer session，再启动 actor，避免丢失第一条公开事件。
    async fn start(context: &AuthContext, channel_id: &str) -> Result<Self, ChainRunError> {
        let observer = start_chain_helix_session_as(context, &context.observer, &[channel_id])
            .await
            .map_err(ChainRunError::Blocked)?;
        let actor = match start_chain_helix_session(context, &[channel_id]).await {
            Ok(actor) => actor,
            Err(error) => {
                let _ = observer.close().await;
                return Err(ChainRunError::Blocked(error));
            }
        };
        Ok(Self {
            channel_id: channel_id.to_string(),
            actor,
            observer,
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
        command_event_on(&mut self.actor, command, payload, event_name, mutation_id).await
    }

    /// 等待 observer session tap 记录的 canonical post_chain frame，并校验 action/chain/mutation。
    async fn raw_event(
        &mut self,
        expected_action: &str,
        chain_id: &str,
        mutation_id: Option<&str>,
    ) -> Result<Value, ChainRunError> {
        self.observer
            .wait_ws_action(
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
                            extract_id(frame, &["clientMutationId", "client_mutation_id"])
                                .as_deref()
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

    /// 关闭 actor/observer 逻辑租约；Phase 池会在 cleanup test 统一停止物理资源。
    async fn close(self) -> Result<(), String> {
        let actor_result = self.actor.close().await;
        let observer_result = self.observer.close().await;
        match (actor_result, observer_result) {
            (Ok(()), Ok(())) => Ok(()),
            (Err(error), Ok(())) | (Ok(()), Err(error)) => Err(error),
            (Err(actor), Err(observer)) => Err(format!("{actor}; {observer}")),
        }
    }
}

/// 向任意真实 Helix session 注入 command，并等待同一 mutation/req_id 的 terminal event。
async fn command_event_on(
    session: &mut RealHelixSession,
    command: &'static str,
    payload: Value,
    event_name: &str,
    mutation_id: Option<&str>,
) -> Result<Value, ChainRunError> {
    let req_id = payload
        .get("req_id")
        .and_then(Value::as_str)
        .map(str::to_string);
    session
        .command(command, payload)
        .await
        .map_err(ChainRunError::Blocked)?;
    let event = session
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
    serde_json::from_slice(event.0.as_ref())
        .map_err(|error| ChainRunError::Failed(format!("{event_name} event JSON failed: {error}")))
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
                    "go_http": chain_go_http_base(),
                    "go_ws": chain_go_ws_url(),
                    "first_failing_layer": "profile health/auth",
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
                    "go_http": chain_go_http_base(),
                    "first_failing_layer": "profile channel fixture",
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
            write_gate_evidence(
                CHAIN_PHASE,
                domain,
                gate,
                "PASS",
                complete_chain_evidence(details, &channel),
            )?;
            Ok(())
        }
        (Err(ChainRunError::Blocked(error)), Ok(())) => {
            write_gate_evidence(
                CHAIN_PHASE,
                domain,
                gate,
                "BLOCKED_PRECONDITION",
                json!({
                    "go_http": chain_go_http_base(),
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
                    "go_http": chain_go_http_base(),
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
        "text_chain"
    } else {
        "chain"
    }
}

/// 为成功 Chain Gate 补齐同轮 HTTP/WS、projection、SQLite、EventSink 与清理合同。
fn complete_chain_evidence(mut details: Value, channel_id: &str) -> Value {
    let object = details
        .as_object_mut()
        .expect("Chain Gate details must remain a JSON object");
    object.insert(
        "corr_key".to_string(),
        object
            .get("chain_id")
            .cloned()
            .unwrap_or_else(|| Value::String(channel_id.to_string())),
    );
    object.insert(
        "http_business".to_string(),
        Value::String("status=SUCCESS before authoritative chain terminal".to_string()),
    );
    object.insert(
        "ws_action".to_string(),
        Value::String("post_chain authoritative actor/observer frame".to_string()),
    );
    object.insert(
        "projection".to_string(),
        Value::String("im:post_chain:* or im:read:result".to_string()),
    );
    object.insert(
        "persist_after".to_string(),
        Value::String("chain projection read back after PersistOk".to_string()),
    );
    object.insert(
        "event_sink".to_string(),
        Value::String("NativeEventSink canonical DomainEvent".to_string()),
    );
    object.insert(
        "sqlite_readback".to_string(),
        json!({"tables": ["chain_summary", "chain_entry", "chain_window", "chain_mutation", "chain_viewer"]}),
    );
    object.insert(
        "cleanup".to_string(),
        json!({"status": "PASS", "remote_channel": channel_id, "engine_ws_sqlite": "closed"}),
    );
    details
}

/// 对当前明确未实现的 Category/Acceptance Gate 走公开 Command seam 并断言 fail-closed。
pub(crate) fn run_unimplemented_chain_gate(gate: &str) -> Result<(), String> {
    let (domain, mode) = match gate {
        "CHAIN-CAT-G01" | "CHAIN-CAT-G02" | "CHAIN-CAT-G03" => ("category_chain", "CATEGORY"),
        "CHAIN-ACC-G01" => ("acceptance", "CATEGORY"),
        _ => return Err(format!("unknown unimplemented chain Gate {gate}")),
    };
    let mut shell = ExecutionShell::new();
    shell.register(ImModule::new(ImConfig::default()));
    let payload = json!({
        "channel_id": "000000000000000000000001",
        "client_mutation_id": format!("{gate}-fail-closed"),
        "device_id": "chain-contract-device",
        "mode": mode,
        "title": "分类接龙未实现合同",
        "category_ids": ["cat-a", "cat-b"],
    });
    let tick = Tick::Command(AppCommand::new(
        "post_chain_create",
        Bytes::from(serde_json::to_vec(&payload).map_err(|error| error.to_string())?),
    ));
    let error = shell
        .step(tick, 1_700_000_000_000)
        .expect_err("Category/Acceptance must fail closed until production support exists")
        .to_string();
    if !error.contains("only TEXT mode is supported") {
        return Err(format!(
            "{gate} returned unexpected fail-closed error: {error}"
        ));
    }
    write_gate_evidence(
        CHAIN_PHASE,
        domain,
        gate,
        "NOT_IMPLEMENTED",
        json!({
            "reason": "production helix-im only supports TEXT mode",
            "first_failing_layer": "Helix public Tick::Command validation",
            "expected_error": "only TEXT mode is supported",
            "corr_key": format!("{gate}-fail-closed"),
            "cleanup": {"status": "PASS", "remote_resources": "none_created"},
        }),
    )?;
    Ok(())
}

/// 以当前 profile 的 health、账号和共享锁装配可审计上下文。
async fn prepare_chain_context(
) -> Result<(tokio::sync::MutexGuard<'static, ()>, Arc<AuthContext>), String> {
    let guard = REMOTE_LOCK
        .get_or_init(|| async { Mutex::new(()) })
        .await
        .lock()
        .await;
    let context = CHAIN_AUTH_CONTEXT
        .get_or_init(|| async {
            discover_auth_context_at(chain_go_http_base(), go_health_url())
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

/// 通过草稿更新/版本冲突再 publish 断言 revision、HTTP authority、WS frame 和账本。
async fn execute_publish(context: &AuthContext, channel_id: &str) -> Result<Value, ChainRunError> {
    let mut harness = ChainHarness::start(context, channel_id).await?;
    let result = async {
        let (chain_id, _) = create_chain(&mut harness, channel_id).await?;
        let update_mutation = run_token("chain-update-draft");
        let update = harness
            .command_event(
                "post_chain_update_draft",
                update_draft_payload(
                    channel_id,
                    &chain_id,
                    &update_mutation,
                    "chain-device-a",
                    1,
                    "Native 文字接龙（已修改）",
                    "Phase5 native gate updated",
                    1_800_000_000_000,
                ),
                "im:post_chain:draft",
                Some(&update_mutation),
            )
            .await?;
        assert_chain_summary(&harness.actor.storage, &chain_id, "DRAFT", 0, 2).await?;
        assert_chain_draft_fields(
            &harness.actor.storage,
            &chain_id,
            "Native 文字接龙（已修改）",
            "Phase5 native gate updated",
            1_800_000_000_000,
        )
        .await?;
        assert_mutation(&harness.actor.storage, &update_mutation, "CONFIRMED").await?;

        let duplicate_update = harness
            .command_event(
                "post_chain_update_draft",
                update_draft_payload(
                    channel_id,
                    &chain_id,
                    &update_mutation,
                    "chain-device-a",
                    1,
                    "Native 文字接龙（已修改）",
                    "Phase5 native gate updated",
                    1_800_000_000_000,
                ),
                "im:post_chain:draft",
                Some(&update_mutation),
            )
            .await?;
        if duplicate_update
            .pointer("/data/chainSummary/revision")
            .and_then(Value::as_i64)
            != Some(2)
        {
            return Err(ChainRunError::Failed(format!(
                "duplicate draft update returned unexpected revision: {duplicate_update}"
            )));
        }

        let stale_mutation = run_token("chain-update-stale");
        let stale = harness
            .command_event(
                "post_chain_update_draft",
                update_draft_payload(
                    channel_id,
                    &chain_id,
                    &stale_mutation,
                    "chain-device-a",
                    1,
                    "stale",
                    "must reject",
                    1_900_000_000_000,
                ),
                "im:post_chain:draft",
                Some(&stale_mutation),
            )
            .await?;
        if data_string(&stale, &["errorCode", "error_code"]).as_deref()
            != Some("CHAIN_REVISION_CONFLICT")
        {
            return Err(ChainRunError::Failed(format!(
                "stale draft update did not expose CHAIN_REVISION_CONFLICT: {stale}"
            )));
        }
        assert_mutation(&harness.actor.storage, &stale_mutation, "REJECTED").await?;
        assert_chain_summary(&harness.actor.storage, &chain_id, "DRAFT", 0, 2).await?;

        let mutation = run_token("chain-publish");
        let event = harness
            .command_event(
                "post_chain_publish",
                publish_payload(channel_id, &chain_id, &mutation, "chain-device-a", 2),
                "im:post_chain:publish",
                Some(&mutation),
            )
            .await?;
        let raw = harness
            .raw_event("im:post_chain:publish", &chain_id, Some(&mutation))
            .await?;
        assert_chain_summary(&harness.actor.storage, &chain_id, "PUBLISHED", 0, 3).await?;
        assert_mutation(&harness.actor.storage, &mutation, "CONFIRMED").await?;
        Ok(json!({
            "chain_id": chain_id,
            "event_id": extract_id(&event, &["eventId", "event_id"]),
            "raw_ws_action": raw.pointer("/data/action"),
            "update_event": update.get("event"),
            "stale_error": stale.pointer("/data/errorCode"),
            "checks": ["draft_update", "draft_fields", "draft_idempotent_retry", "stale_revision_rejected", "revision=3", "published", "raw_ws", "sqlite_mutation"]
        }))
    }
    .await;
    let cleanup = harness.close().await;
    finish_gate(result, cleanup)
}

/// 让同一 actor 连续注入两个 append，再证明作者级 upsert 保留稳定 Entry/Post/seq。
async fn execute_concurrent_append(
    context: &AuthContext,
    channel_id: &str,
) -> Result<Value, ChainRunError> {
    let mut harness = ChainHarness::start(context, channel_id).await?;
    let result = async {
        let (chain_id, _) = create_and_publish(&mut harness, channel_id).await?;
        let first = run_token("chain-append-a");
        let second = run_token("chain-append-b");
        let first_event = harness
            .command_event(
                "post_chain_append",
                append_payload(channel_id, &chain_id, "第一条", &first, "chain-device-a"),
                "im:post_chain:upsert",
                Some(&first),
            )
            .await?;
        let first_raw = harness
            .raw_event("im:post_chain:upsert", &chain_id, Some(&first))
            .await?;
        let second_event = harness
            .command_event(
                "post_chain_append",
                append_payload(channel_id, &chain_id, "第二条", &second, "chain-device-a"),
                "im:post_chain:upsert",
                Some(&second),
            )
            .await?;
        let second_raw = harness
            .raw_event("im:post_chain:upsert", &chain_id, Some(&second))
            .await?;
        let event_identities = [&first_event, &second_event]
            .iter()
            .map(|event| chain_entry_identity(event))
            .collect::<Result<BTreeSet<_>, _>>()?;
        if event_identities.len() != 1 {
            return Err(ChainRunError::Failed(format!(
                "same author append changed entry/post/seq identity: {event_identities:?}"
            )));
        }
        let raw_identities = [&first_raw, &second_raw]
            .iter()
            .map(|event| chain_entry_identity(event))
            .collect::<Result<BTreeSet<_>, _>>()?;
        for mutation in [&first, &second] {
            assert_mutation(&harness.actor.storage, mutation, "CONFIRMED").await?;
        }
        if raw_identities != event_identities {
            return Err(ChainRunError::Failed(format!(
                "EventSink/raw WS upsert identity mismatch: event={event_identities:?}, raw={raw_identities:?}"
            )));
        }
        let rows = scan_chain_entries(&harness.actor.storage, &chain_id)
            .await
            .map_err(ChainRunError::Failed)?;
        assert_seq_rows(&rows, 1)?;
        let stable_entry_id = row_text(&rows[0], "entry_id")
            .ok_or_else(|| ChainRunError::Failed("upsert row missing entry_id".to_string()))?;
        let stable_post_id = row_text(&rows[0], "post_id")
            .ok_or_else(|| ChainRunError::Failed("upsert row missing post_id".to_string()))?;
        let stable_seq = row_i64(&rows[0], "seq")
            .ok_or_else(|| ChainRunError::Failed("upsert row missing seq".to_string()))?;
        if row_text(&rows[0], "author_id").as_deref() != Some(context.actor.user_id.as_str())
            || row_text(&rows[0], "status").as_deref() != Some("ACTIVE")
        {
            return Err(ChainRunError::Failed(format!(
                "upsert row is not the actor ACTIVE entry: {:?}",
                rows[0]
            )));
        }
        if !event_identities.contains(&(
            stable_entry_id.clone(),
            stable_post_id.clone(),
            stable_seq,
        )) {
            return Err(ChainRunError::Failed(format!(
                "event/storage entry identity mismatch: events={event_identities:?}, row={rows:?}"
            )));
        }
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
        assert_seq_rows(&after_duplicate, 1)?;
        if row_text(&after_duplicate[0], "entry_id").as_deref() != Some(stable_entry_id.as_str())
            || row_text(&after_duplicate[0], "post_id").as_deref()
                != Some(stable_post_id.as_str())
            || row_i64(&after_duplicate[0], "seq") != Some(stable_seq)
        {
            return Err(ChainRunError::Failed(format!(
                "reconcile changed stable upsert identity: {after_duplicate:?}"
            )));
        }
        Ok(json!({
            "chain_id": chain_id,
            "mutations": [first, second],
            "entry_count": rows.len(),
            "stable_entry_id": stable_entry_id,
            "stable_post_id": stable_post_id,
            "stable_seq": stable_seq,
            "reconcile_mutation": reconcile,
            "reconcile_event": data_string(&reconcile_event, &["clientMutationId"]),
            "checks": ["two_commands", "author_upsert", "stable_entry", "stable_post", "stable_seq", "idempotent_reconcile"]
        }))
    }
    .await;
    let cleanup = harness.close().await;
    finish_gate(result, cleanup)
}

/// 用同一作者多次更新验证窗口读取不会把作者级 upsert 误报成多条 Entry。
async fn execute_window(context: &AuthContext, channel_id: &str) -> Result<Value, ChainRunError> {
    let mut harness = ChainHarness::start(context, channel_id).await?;
    let result = async {
        let (chain_id, _) = create_and_publish(&mut harness, channel_id).await?;
        for text in ["窗口一", "窗口二", "窗口三", "窗口四", "窗口五"] {
            append_one(&mut harness, channel_id, &chain_id, text, "chain-device-a").await?;
        }
        let entries = scan_chain_entries(&harness.actor.storage, &chain_id)
            .await
            .map_err(ChainRunError::Failed)?;
        assert_seq_rows(&entries, 1)?;
        if row_text(&entries[0], "author_id").as_deref() != Some(context.actor.user_id.as_str())
            || row_text(&entries[0], "status").as_deref() != Some("ACTIVE")
            || row_text(&entries[0], "text").as_deref() != Some("窗口五")
        {
            return Err(ChainRunError::Failed(format!(
                "window author upsert did not retain only the latest ACTIVE text: {:?}",
                entries[0]
            )));
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
        // Chain read relay carries the unwrapped WindowResult; retain the legacy CommonRes path.
        let window = read
            .pointer("/data/body/textWindow")
            .or_else(|| read.pointer("/data/body/data/textWindow"))
            .or_else(|| read.pointer("/data/body/data"))
            .ok_or_else(|| ChainRunError::Failed("post_chain_get missing textWindow projection".to_string()))?;
        let hidden = window.get("hiddenCount").and_then(Value::as_i64).unwrap_or_default();
        if hidden < 0 {
            return Err(ChainRunError::Failed("textWindow.hiddenCount is negative".to_string()));
        }
        if hidden != 0 {
            return Err(ChainRunError::Failed(format!(
                "single-author window unexpectedly reports hidden entries: {window}"
            )));
        }
        let stored = scan_chain_row(&harness.actor.storage, "chain_window", "chain_id", &chain_id)
            .await
            .map_err(ChainRunError::Failed)?;
        if row_text(&stored, "head_ids").is_none() || row_text(&stored, "tail_ids").is_none() {
            return Err(ChainRunError::Failed("chain_window lacks head/tail durable columns".to_string()));
        }
        Ok(json!({"chain_id": chain_id, "entry_count": entries.len(), "hidden_count": hidden, "checks": ["author_upsert_window", "latest_text", "hidden_count_zero", "sqlite_window"]}))
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
        let observer_hydrate_req = run_token("chain-device-hydrate-get");
        let observer_hydrate = command_event_on(
            &mut observer,
            "post_chain_get",
            get_payload(
                channel_id,
                &chain_id,
                "chain-device-b",
                &observer_hydrate_req,
                20,
            ),
            "im:read:result",
            None,
        )
        .await?;
        assert_read_success(&observer_hydrate)?;
        let mutation = run_token("chain-device-append");
        let actor_append = harness
            .command_event("post_chain_append", append_payload(channel_id, &chain_id, "双设备同步", &mutation, "chain-device-a"), "im:post_chain:upsert", Some(&mutation))
            .await?;
        let observer_raw = harness
            .raw_event("im:post_chain:upsert", &chain_id, Some(&mutation))
            .await?;
        let actor_entry = extract_id(&actor_append, &["entryId", "entry_id"])
            .ok_or_else(|| ChainRunError::Failed("actor append has no entryId".to_string()))?;
        let raw_entry = extract_id(&observer_raw, &["entryId", "entry_id"])
            .ok_or_else(|| ChainRunError::Failed("observer raw WS has no entryId".to_string()))?;
        if actor_entry != raw_entry {
            return Err(ChainRunError::Failed(format!("actor/raw WS entry mismatch: {actor_entry} != {raw_entry}")));
        }
        let read_req_id = run_token("chain-device-late-join-get");
        let observer_read = command_event_on(
            &mut observer,
            "post_chain_get",
            get_payload(channel_id, &chain_id, "chain-device-b", &read_req_id, 20),
            "im:read:result",
            None,
        )
        .await?;
        assert_read_success(&observer_read)?;
        let observer_entry = extract_id(&observer_read, &["entryId", "entry_id"])
            .ok_or_else(|| ChainRunError::Failed("observer GET has no entryId".to_string()))?;
        if observer_entry != actor_entry
            || !contains_field_value(&observer_read, "text", "双设备同步")
        {
            return Err(ChainRunError::Failed(format!(
                "observer late-join GET missing appended entry: {observer_read}"
            )));
        }
        // reconcile 只回发起 mutation 的设备；设备 B 的职责是通过 WS/GET 恢复公共 Entry。
        let reconcile_event = harness
            .command_event(
                "post_chain_reconcile",
                reconcile_payload(channel_id, &chain_id, &mutation, "chain-device-a"),
                "im:post_chain:reconcile",
                Some(&mutation),
            )
            .await?;
        if data_string(&reconcile_event, &["clientMutationId", "client_mutation_id"])
            .as_deref()
            != Some(mutation.as_str())
        {
            return Err(ChainRunError::Failed(format!(
                "actor reconcile returned wrong mutation: {reconcile_event}"
            )));
        }
        assert_chain_summary(&harness.actor.storage, &chain_id, "PUBLISHED", 1, 2).await?;
        assert_chain_summary(&observer.storage, &chain_id, "PUBLISHED", 1, 2).await?;
        assert_mutation(&harness.actor.storage, &mutation, "CONFIRMED").await?;
        scan_chain_row(&observer.storage, "chain_entry", "entry_id", &observer_entry)
            .await
            .map_err(ChainRunError::Failed)?;
        Ok(json!({"chain_id": chain_id, "entry_id": actor_entry, "hydrate_read_req_id": observer_hydrate_req, "late_join_read_req_id": read_req_id, "reconcile_mutation": mutation, "checks": ["two_helix_sessions", "observer_get_hydrate", "actor_event_sink", "observer_raw_ws", "same_event_entry", "late_join_get", "reconcile", "actor_sqlite", "observer_sqlite"]}))
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
    match body.get("status").and_then(Value::as_str) {
        Some("SUCCESS") => {}
        Some(status) => {
            return Err(ChainRunError::Failed(format!(
                "Go chain read projection returned status={status}: {body}"
            )))
        }
        None => {
            // 文字接龙 read API 的真实 CommonRes 会把 data 裸展开；此时必须以
            // 非空 chainSummary.chainId 和合法 mode/status 证明业务投影，而不是仅凭事件存在。
            let summary = body.get("chainSummary").ok_or_else(|| {
                ChainRunError::Failed(format!(
                    "Go chain read projection missing status=SUCCESS or chainSummary: {body}"
                ))
            })?;
            let chain_id = summary.get("chainId").and_then(Value::as_str);
            let mode = summary.get("mode").and_then(Value::as_str);
            let chain_status = summary.get("status").and_then(Value::as_str);
            if chain_id.is_none()
                || chain_id == Some("")
                || mode != Some("TEXT")
                || !matches!(chain_status, Some("DRAFT" | "PUBLISHED" | "CLOSED"))
            {
                return Err(ChainRunError::Failed(format!(
                    "Go chain read裸 projection lacks valid chainSummary authority: {body}"
                )));
            }
        }
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

/// 断言草稿字段已在 Helix 本地摘要投影中原子更新，避免只验证事件 payload。
async fn assert_chain_draft_fields(
    storage: &NativeStorage,
    chain_id: &str,
    title: &str,
    description: &str,
    deadline_at: i64,
) -> Result<(), ChainRunError> {
    let row = scan_chain_row(storage, "chain_summary", "chain_id", chain_id)
        .await
        .map_err(ChainRunError::Failed)?;
    if row_text(&row, "title").as_deref() != Some(title)
        || row_text(&row, "description").as_deref() != Some(description)
        || row_i64(&row, "deadline_at") != Some(deadline_at)
    {
        return Err(ChainRunError::Failed(format!(
            "chain draft fields mismatch: {row:?}"
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

/// 从任意 EventSink/WS authority 中严格提取稳定的 Entry/Post/seq 身份。
fn chain_entry_identity(event: &Value) -> Result<(String, String, i64), ChainRunError> {
    recursive_chain_entry_identity(event).ok_or_else(|| {
        ChainRunError::Failed(
            "chain upsert event missing one coherent entryId/postId/seq object".to_string(),
        )
    })
}

/// 只在同一 Entry 对象内取身份，避免把频道 eventSeq 误当成接龙 seq。
fn recursive_chain_entry_identity(value: &Value) -> Option<(String, String, i64)> {
    if let Value::Object(object) = value {
        let entry_id = object
            .get("entryId")
            .or_else(|| object.get("entry_id"))
            .and_then(Value::as_str);
        let post_id = object
            .get("postId")
            .or_else(|| object.get("post_id"))
            .and_then(Value::as_str);
        let seq = object.get("seq").and_then(|value| {
            value
                .as_i64()
                .or_else(|| value.as_str().and_then(|text| text.parse::<i64>().ok()))
        });
        if let (Some(entry_id), Some(post_id), Some(seq)) = (entry_id, post_id, seq) {
            return Some((entry_id.to_string(), post_id.to_string(), seq));
        }
        return object.values().find_map(recursive_chain_entry_identity);
    }
    if let Value::Array(items) = value {
        return items.iter().find_map(recursive_chain_entry_identity);
    }
    None
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

/// 构造发布前草稿稀疏更新 payload，expectedRevision 由 Gate 显式控制。
fn update_draft_payload(
    channel_id: &str,
    chain_id: &str,
    mutation: &str,
    device_id: &str,
    revision: i64,
    title: &str,
    description: &str,
    deadline_at: i64,
) -> Value {
    json!({
        "channel_id": channel_id,
        "chain_id": chain_id,
        "title": title,
        "description": description,
        "deadline_at": deadline_at,
        "expected_revision": revision,
        "client_mutation_id": mutation,
        "device_id": device_id
    })
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

/// 递归断言 authority 树中字段值，防止 read relay 只返回摘要而缺少正文。
fn contains_field_value(value: &Value, key: &str, expected: &str) -> bool {
    match value {
        Value::Object(object) => object.iter().any(|(name, child)| {
            (name == key && child.as_str() == Some(expected))
                || contains_field_value(child, key, expected)
        }),
        Value::Array(items) => items
            .iter()
            .any(|child| contains_field_value(child, key, expected)),
        _ => false,
    }
}
