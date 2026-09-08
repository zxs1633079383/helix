use crate::shared;
use helix_core::effect::{ScanSpec, SqlValue};
use helix_core::ports::Storage;
use helix_driver_native::NativeStorage;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use tokio::time::{sleep, Duration, Instant};

const GATE: &str = "MV3-G10e";
const ENDPOINT: &str = "/channel/member/change/notify";

/// 等待频道启动期 authority 写入静止，避免迟到的 increment snapshot 覆盖 notify 状态。
async fn wait_stable_notify_baseline(
    storage: &NativeStorage,
    channel: &str,
    wait_for: Duration,
) -> Result<(i64, bool), String> {
    let deadline = Instant::now() + wait_for;
    let mut previous = None;
    let mut stable_since: Option<Instant> = None;
    loop {
        let rows = storage
            .scan(ScanSpec {
                table: "channel",
                limit: Some(4),
                filter: Some(("id", SqlValue::Text(channel.to_string()))),
                order_by: &[],
            })
            .await
            .map_err(|error| format!("scan channel notify baseline failed: {error}"))?;
        let row = rows
            .into_iter()
            .next()
            .ok_or_else(|| format!("channel notify baseline row {channel} not found"))?;
        let unread = shared::row_i64(&row, "unread_count").ok_or_else(|| {
            format!("channel notify baseline unread_count is not integer: {row:?}")
        })?;
        let is_top = shared::row_i64(&row, "is_top") == Some(1);
        let current = (unread, is_top);
        if previous == Some(current) {
            if let Some(started) = stable_since {
                // channel/create emits a delayed join NOTICE update; keep the baseline quiet
                // long enough that the notify assertion compares against the settled authority.
                if started.elapsed() >= Duration::from_secs(4) {
                    return Ok(current);
                }
            }
        } else {
            previous = Some(current);
            stable_since = Some(Instant::now());
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "channel notify baseline did not settle: channel={channel}, current={current:?}"
            ));
        }
        sleep(Duration::from_millis(100)).await;
    }
}

/// 真实 HTTP→Go WS→Helix Persist→SQLite→EventSink 链上回放一种通知状态。
async fn exercise_notify_chain(
    context: &shared::AuthContext,
    channel: &str,
    session: &mut shared::RealHelixSession,
) -> Result<Value, String> {
    // 隔离 session 要现场等待恢复；pooled session 创建时已在共享 runtime 完成一次恢复闸门。
    if !session.is_pooled() {
        session
            .wait_event("im:sync:recovered", |_| true, Duration::from_secs(20))
            .await
            .map_err(|error| {
                format!("initial recovery did not settle before notify mutation: {error}")
            })?;
    }
    let http = shared::go_http(&context.actor).await?;
    let mut states = Vec::new();

    let result = async {
        for notify in ["NORMAL", "STRONG", "IGNORE"] {
            // 先等待启动期 authority 静止，再读取每个状态前的基线；否则 create 产生的
            // 迟到 join post 会被误判为 notify 改写。
            let (baseline_unread, baseline_top) = wait_stable_notify_baseline(
                &session.storage,
                channel,
                Duration::from_secs(20),
            )
            .await?;
            let req_id = shared::run_token("g10e-notify");
            // 通过公开 Command seam 触发真实 NativeHttp；测试不直接 curl 写接口，避免绕过 Helix。
            session
                .command(
                    "im_channel_member_notify",
                    json!({"channel_id": channel, "notify": notify, "req_id": req_id}),
                )
                .await?;
            let ws_frame = session
                .wait_ws_action(
                "update_channel",
                |frame| {
                    frame.pointer("/data/id").and_then(Value::as_str) == Some(channel)
                        && frame.pointer("/data/userId").and_then(Value::as_str)
                            == Some(context.actor.user_id.as_str())
                        && frame.pointer("/data/notify").and_then(Value::as_str) == Some(notify)
                },
                Duration::from_secs(25),
            )
            .await?;
            let seen_events = Arc::new(Mutex::new(Vec::new()));
            let seen_events_for_predicate = Arc::clone(&seen_events);
            let event = session
                .wait_event(
                    "im:channel:update",
                    |envelope| {
                        if let Ok(mut seen) = seen_events_for_predicate.lock() {
                            if seen.len() < 32 {
                                seen.push(envelope.clone());
                            }
                        }
                        shared::data_string(envelope, &["id", "channelId", "channel_id"]).as_deref()
                            == Some(channel)
                            && envelope
                                .pointer("/data/notifyProps")
                                .and_then(Value::as_str)
                                == Some(notify)
                    },
                    Duration::from_secs(25),
                )
                .await
                .map_err(|error| {
                    format!(
                        "{error}; matching Go WS frame was observed but Helix projection was not: {ws_frame}; seen_events={} ",
                        seen_events.lock().map(|seen| Value::Array(seen.clone())).unwrap_or(Value::Null)
                    )
                })?;
            let event_json: Value = serde_json::from_slice(event.0.as_ref())
                .map_err(|error| format!("decode {notify} Helix event: {error}"))?;
            let unread_unchanged = event_json.pointer("/data/unreadCount")
                == Some(&json!(baseline_unread));
            // Notify-only update_channel frames intentionally omit channelIsTop; when the
            // sparse field is present, it must still agree with the settled SQLite baseline.
            let top_unchanged = event_json
                .pointer("/data/channelIsTop")
                .map_or(true, |value| value == &json!(baseline_top));
            if !unread_unchanged || !top_unchanged {
                return Err(format!(
                    "notify={notify} changed unread/top in Helix event (baseline unread={baseline_unread}, top={baseline_top}): {event_json}"
                ));
            }
            // Command 写族不回灌 HTTP body；用同一身份的只读 HTTP query 证明 Go 已提交该状态。
            let query = shared::request_json(
                &http,
                "POST",
                &format!("{}/channel/query", shared::go_http_base()),
                Some(json!({
                    "id": channel,
                    "pageNumber": 1,
                    "pageSize": 1,
                    "offset": 0
                })),
                "g10e-notify-http-readback",
            )
            .await?;
            let query_body = shared::require_success(query, "channel/query")?;
            let query_row = match query_body.pointer("/data") {
                Some(Value::Array(rows)) => rows
                    .iter()
                    .find(|row| {
                        shared::string_field(row, &["id", "channelId", "channel_id"])
                            .as_deref()
                            == Some(channel)
                    })
                    .ok_or_else(|| {
                        format!("Go channel/query omitted notify channel {channel}: {query_body}")
                    })?,
                Some(Value::Object(_)) => query_body
                    .pointer("/data")
                    .ok_or_else(|| format!("Go channel/query data disappeared: {query_body}"))?,
                _ => {
                    return Err(format!(
                        "Go channel/query data is not a channel row: {query_body}"
                    ))
                }
            };
            let (channel_row, member_row) = shared::wait_channel_notify(
                &session.storage,
                channel,
                &context.actor.user_id,
                notify,
                Duration::from_secs(8),
            )
            .await?;
            states.push(json!({
                "notify": notify,
                "http": {
                    "endpoint": ENDPOINT,
                    "method": "POST",
                    "command": "im_channel_member_notify",
                    "requestId": req_id,
                    "admission": "fire-and-forget command; terminal state asserted by Go query"
                },
                "httpReadback": {
                    "endpoint": "/channel/query",
                    "status": "SUCCESS",
                    "channelId": shared::string_field(query_row, &["id", "channelId", "channel_id"]),
                    "channelQueryIsReadback": true
                },
                "ws": ws_frame,
                "helixEvent": event_json,
                "sqlite": {
                    "channelNotifyProps": shared::row_text(&channel_row, "notify_props"),
                    "memberNotify": shared::row_text(&member_row, "notify"),
                    "unreadCount": shared::row_i64(&channel_row, "unread_count"),
                    "channelIsTop": shared::row_i64(&channel_row, "is_top"),
                    "baselineUnreadCount": baseline_unread,
                    "baselineChannelIsTop": baseline_top,
                }
            }));
        }

        // Re-enter the local query path after the last state; it must expose the same scalar.
        session
            .command(
                "im_query_dialog_list",
                json!({"req_id": "g10e-native-reentry"}),
            )
            .await?;
        let query = session
            .wait_event(
                "im:read:result",
                |envelope| {
                    envelope.pointer("/data/req_id").and_then(Value::as_str)
                        == Some("g10e-native-reentry")
                        && envelope
                            .pointer("/data/body/items")
                            .and_then(Value::as_array)
                            .is_some_and(|items| {
                                items.iter().any(|item| {
                                    shared::string_field(item, &["id", "channelId", "channel_id"])
                                        .as_deref()
                                        == Some(channel)
                                        && item
                                            .get("notifyProps")
                                            .and_then(Value::as_str)
                                            == Some("IGNORE")
                                })
                            })
                },
                Duration::from_secs(15),
            )
            .await?;
        let query_json: Value = serde_json::from_slice(query.0.as_ref())
            .map_err(|error| format!("decode G10e query result: {error}"))?;
        Ok::<Value, String>(json!({"states": states, "reentry": query_json}))
    }
    .await;
    result
}

/// 删除并核验本轮重开数据库及sidecar，残留或无法检查均不计清理成功。
fn remove_restart_storage(db_path: &std::path::Path) -> Result<(), String> {
    shared::remove_test_storage(db_path);
    for suffix in ["", "-wal", "-shm"] {
        let path = std::path::PathBuf::from(format!("{}{suffix}", db_path.display()));
        if path
            .try_exists()
            .map_err(|error| format!("SQLite cleanup inspection failed: {error}"))?
        {
            return Err(format!("SQLite cleanup left file: {}", path.display()));
        }
    }
    Ok(())
}

/// 重开真实SQLite并校验通知值；所有读回错误先释放句柄与文件，再交还调用方。
async fn verify_restart(db_path: &std::path::Path, channel: &str) -> Result<Value, String> {
    let result = async {
        let reopened = NativeStorage::open(&format!("sqlite:{}?mode=rw", db_path.display()))
            .await
            .map_err(|error| format!("reopen SQLite failed: {error}"))?;
        let rows = reopened
            .scan(ScanSpec {
                table: "channel",
                limit: Some(4),
                filter: Some(("id", SqlValue::Text(channel.to_string()))),
                order_by: &[],
            })
            .await
            .map_err(|error| format!("restart readback failed: {error}"))?;
        let row = rows
            .first()
            .ok_or_else(|| "restart channel row missing".to_string())?;
        let notify = shared::row_text(row, "notify_props");
        if notify.as_deref() != Some("IGNORE") {
            return Err(format!(
                "restart notify_props mismatch: expected=IGNORE actual={notify:?}"
            ));
        }
        Ok(json!({"channelNotifyProps": notify, "channelId": channel}))
    }
    .await;
    shared::finish_gate_cleanup(result, remove_restart_storage(db_path), Ok(()))
}

/// MV3-G10e：三态通知完整终态与重启读回都成功、且清理完成后才发布PASS。
#[tokio::test]
async fn mv3_g10e_real_chain() {
    let (_guard, context) = shared::remote_setup().await;
    let channel = shared::create_channel(&context)
        .await
        .unwrap_or_else(|error| panic!("{GATE} channel setup failed: {error}"));
    let mut stage = "session_setup";
    let mut session_cleanup = json!({"status": "not_started"});
    let result = async {
        let mut session = shared::start_real_helix_session(&context, &[channel.as_str()]).await?;
        stage = "notify_chain";
        let business = async {
            shared::insert_channel_fixture(
                &session.storage,
                &channel,
                &context.actor.company_id,
                &context.actor.user_id,
                "P",
                "",
                &[context.actor.user_id.as_str()],
            )
            .await?;
            exercise_notify_chain(&context, &channel, &mut session).await
        }
        .await;
        // 无论业务是否成功都停止engine，失败也保留关闭错误，并移除本轮SQLite。
        let owned_db_path = session.storage_path().to_path_buf();
        let shutdown = session.close_preserving_db().await;
        session_cleanup = json!({"ok": shutdown.is_ok(), "error": shutdown.as_ref().err()});
        match (business, shutdown) {
            (Ok(mut evidence), Ok(db_path)) => {
                stage = "restart_readback";
                evidence["restartReadback"] = verify_restart(&db_path, &channel).await?;
                Ok(evidence)
            }
            (Err(error), Ok(db_path)) => {
                shared::finish_gate_cleanup(Err(error), remove_restart_storage(&db_path), Ok(()))
            }
            (result, Err(error)) => shared::finish_gate_cleanup(
                result,
                Err(error),
                remove_restart_storage(&owned_db_path),
            ),
        }
    }
    .await;
    let cleanup = shared::close_channel(&context, &channel).await;
    let cleanup_evidence = json!({
        "session": session_cleanup,
        "channel": {"ok": cleanup.is_ok(), "error": cleanup.as_ref().err()},
        "physical_scope": "engine stopped; pooled channels finalized at Phase cleanup",
    });
    let result = shared::finish_gate_cleanup(result, Ok(()), cleanup);
    match result {
        Ok(evidence) => {
            shared::write_gate_evidence("Phase3", "Channel", GATE, "PASS", json!({
                "corr_key": channel,
                "entry": "Tick::Command → NativeHttp → Go HTTP/WS → Persist → EventSink",
                "command_contract": "im_channel_member_notify",
                "go_http": ENDPOINT,
                "http_business": "Go notify mutation reached terminal projections for NORMAL, STRONG and IGNORE",
                "ws_action": "update_channel",
                "terminal_event": "im:channel:update",
                "projection": evidence["reentry"],
                "event_sink": "NativeEventSink::broadcast", "event_sink_required": true,
                "formal_notify_shape": "data.notifyProps scalar",
                "persist_after": evidence["restartReadback"], "persist_required": true,
                "sqlite_readback": evidence["restartReadback"], "sqlite_required": true,
                "chain": evidence,
                "cleanup": "engine/WS closed, SQLite restart verified and removed, channel cleanup completed"
            })).unwrap_or_else(|error| panic!("{GATE} evidence failed: {error}"));
        }
        Err(error) => {
            shared::write_gate_evidence(
                "Phase3",
                "Channel",
                GATE,
                if stage == "session_setup" {
                    "BLOCKED_PRECONDITION"
                } else {
                    "TEST_FAILED"
                },
                json!({
                    "corr_key": channel,
                    "reason": "notify lifecycle or restart assertion failed",
                    "go_http": ENDPOINT, "error": error,
                    "first_failing_layer": stage,
                    "cleanup": cleanup_evidence
                }),
            )
            .unwrap_or_else(|write_error| panic!("{GATE} evidence failed: {write_error}"));
            panic!("{GATE} FAIL: {error}");
        }
    }
}
