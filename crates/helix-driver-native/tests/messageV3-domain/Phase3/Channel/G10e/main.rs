use crate::shared;
use helix_core::effect::{ScanSpec, SqlValue};
use helix_core::ports::Storage;
use helix_driver_native::NativeStorage;
use serde_json::{json, Value};
use tokio::time::Duration;

const GATE: &str = "MV3-G10e";
const ENDPOINT: &str = "/api/cses/channel/member/change/notify";

/// 真实 HTTP→Go WS→Helix Persist→SQLite→EventSink 链上回放一种通知状态。
async fn exercise_notify_chain(
    context: &shared::AuthContext,
    channel: &str,
    session: &mut shared::RealHelixSession,
) -> Result<Value, String> {
    let mut raw_ws = shared::open_raw_ws(&context.actor, 700).await?;
    shared::wait_ws_hello(&mut raw_ws).await?;
    let http = shared::go_http(&context.actor).await?;
    let mut states = Vec::new();

    // 保留未读/置顶基线，证明 notify-only 不会顺带改写这些列。
    let baseline_sql: &'static str = Box::leak(
        format!("UPDATE channel SET unread_count = 7, is_top = 1 WHERE id = '{channel}'")
            .into_boxed_str(),
    );
    session
        .storage
        .execute_raw(baseline_sql)
        .await
        .map_err(|error| format!("seed channel unread/top baseline failed: {error}"))?;

    let result = async {
        for notify in ["NORMAL", "STRONG", "IGNORE"] {
            let observation = shared::request_json(
                &http,
                "POST",
                &format!("{}{}", shared::GO_HTTP_BASE, ENDPOINT),
                Some(json!({"channelId": channel, "notify": notify})),
                "g10e-notify",
            )
            .await?;
            let response = shared::require_success(observation, "channel/member/change/notify")?;
            let ws_frame = shared::wait_ws_action(
                &mut raw_ws,
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
            let event = session
                .wait_event(
                    "im:channel:update",
                    |envelope| {
                        shared::data_string(envelope, &["id", "channelId", "channel_id"]).as_deref()
                            == Some(channel)
                            && envelope
                                .pointer("/data/notifyProps")
                                .and_then(Value::as_str)
                                == Some(notify)
                    },
                    Duration::from_secs(25),
                )
                .await?;
            let event_json: Value = serde_json::from_slice(event.0.as_ref())
                .map_err(|error| format!("decode {notify} Helix event: {error}"))?;
            if event_json.pointer("/data/unreadCount") != Some(&json!(7))
                || event_json.pointer("/data/channelIsTop") != Some(&json!(true))
            {
                return Err(format!(
                    "notify={notify} changed unread/top in Helix event: {event_json}"
                ));
            }
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
                "http": {"endpoint": ENDPOINT, "status": response["status"]},
                "ws": ws_frame,
                "helixEvent": event_json,
                "sqlite": {
                    "channelNotifyProps": shared::row_text(&channel_row, "notify_props"),
                    "memberNotify": shared::row_text(&member_row, "notify"),
                    "unreadCount": shared::row_text(&channel_row, "unread_count"),
                    "channelIsTop": shared::row_text(&channel_row, "is_top"),
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
                            .pointer("/data/body/items/0/id")
                            .and_then(Value::as_str)
                            == Some(channel)
                        && envelope
                            .pointer("/data/body/items/0/notifyProps")
                            .and_then(Value::as_str)
                            == Some("IGNORE")
                },
                Duration::from_secs(15),
            )
            .await?;
        let query_json: Value = serde_json::from_slice(query.0.as_ref())
            .map_err(|error| format!("decode G10e query result: {error}"))?;
        Ok::<Value, String>(json!({"states": states, "reentry": query_json}))
    }
    .await;
    let _ = raw_ws.transport.close().await;
    result
}

/// MV3-G10e：三态免打扰/强提醒真实链，含重启后 SQLite 读回。
#[tokio::test]
async fn mv3_g10e_real_chain() {
    let (_guard, context) = shared::remote_setup().await;
    let channel = shared::create_channel(&context)
        .await
        .unwrap_or_else(|error| panic!("MV3-G10e channel setup failed: {error}"));
    let mut session = match shared::start_real_helix_session(&context, &[channel.as_str()]).await {
        Ok(session) => session,
        Err(error) => {
            shared::write_gate_evidence(
                "Phase3",
                "Channel",
                GATE,
                "BLOCKED_PRECONDITION",
                json!({"reason": "real Helix session unavailable", "first_failing_layer": "Helix native transport/auth", "go_http": ENDPOINT, "error": error}),
            )
            .unwrap_or_else(|write_error| panic!("MV3-G10e evidence failed: {write_error}"));
            let _ = shared::close_channel(&context, &channel).await;
            return;
        }
    };
    shared::insert_channel_fixture(
        &session.storage,
        &channel,
        &context.actor.company_id,
        &context.actor.user_id,
        "P",
        "",
        &[context.actor.user_id.as_str()],
    )
    .await
    .unwrap_or_else(|error| panic!("MV3-G10e channel fixture failed: {error}"));

    match exercise_notify_chain(&context, &channel, &mut session).await {
        Ok(mut evidence) => {
            let db_path = session
                .close_preserving_db()
                .await
                .unwrap_or_else(|error| panic!("MV3-G10e session shutdown failed: {error}"));
            let db_url = format!("sqlite:{}?mode=rwc", db_path.display());
            let reopened = NativeStorage::open(&db_url)
                .await
                .unwrap_or_else(|error| panic!("MV3-G10e reopen SQLite failed: {error}"));
            let rows = reopened
                .scan(ScanSpec {
                    table: "channel",
                    limit: Some(4),
                    filter: Some(("id", SqlValue::Text(channel.clone()))),
                    order_by: &[],
                })
                .await
                .unwrap_or_else(|error| panic!("MV3-G10e restart readback failed: {error}"));
            let row = rows
                .into_iter()
                .next()
                .unwrap_or_else(|| panic!("MV3-G10e restart channel row missing"));
            assert_eq!(
                shared::row_text(&row, "notify_props").as_deref(),
                Some("IGNORE")
            );
            evidence["restartReadback"] = json!({
                "channelNotifyProps": shared::row_text(&row, "notify_props"),
                "channelId": channel,
            });
            shared::remove_test_storage(&db_path);
            shared::write_gate_evidence(
                "Phase3",
                "Channel",
                GATE,
                "PASS",
                json!({"entry": "NativeHttp POST", "command_contract": "im_channel_member_notify", "go_http": ENDPOINT, "terminal_event": "im:channel:update", "formal_notify_shape": "data.notifyProps scalar", "chain": evidence}),
            )
            .unwrap_or_else(|error| panic!("MV3-G10e evidence failed: {error}"));
        }
        Err(error) => {
            let _ = session.close().await;
            shared::write_gate_evidence(
                "Phase3",
                "Channel",
                GATE,
                "BLOCKED_PRECONDITION",
                json!({"reason": "notify terminal chain not observed", "first_failing_layer": "Go notify route/WS or Helix Persist/readback", "go_http": ENDPOINT, "error": error}),
            )
            .unwrap_or_else(|write_error| panic!("MV3-G10e blocked evidence failed: {write_error}"));
        }
    }
    let _ = shared::close_channel(&context, &channel).await;
}
