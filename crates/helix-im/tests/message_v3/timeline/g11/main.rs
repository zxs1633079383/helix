// 回归锚: Spec 047/T058-T059，G-11 已缓存消息定位。

use serde_json::{json, Value};

#[path = "../../support/artifact.rs"]
mod artifact;
#[path = "../g10d/main.rs"]
mod base;

type TestResult<T> = Result<T, String>;

/// 记录定位 persona 的真实 terminal event 与输入。
struct PersonaTrace {
    events: Vec<Value>,
    ticks: Vec<Value>,
}

/// 通过真实首屏和 locate authority 生成活动窗口。
fn run_success() -> TestResult<PersonaTrace> {
    let mut replay = base::base::Replay::new()?;
    replay.attach_full_window()?;
    base::locate_away_from_bottom(&mut replay)?;
    Ok(PersonaTrace {
        events: replay.events()?,
        ticks: vec![json!({
            "kind": "Command",
            "value": {"name": "im_locate_message", "payload": {
                "channel_id": "chfixx0000000000000000001c",
                "message_id": "srvfix0000000000000000000a",
                "pageSize": 20,
                "req_id": "g10d-locate-request",
                "navigationToken": "g10d-navigation"
            }},
            "now_ms": 1_700_001_300_003_i64
        })],
    })
}

/// G-11 定位终态通过 reqId 返回 bounded window，中心滚动由消费者执行。
#[test]
fn g11_locates_target_in_active_bounded_window() -> TestResult<()> {
    let trace = run_success()?;
    assert_eq!(trace.events.len(), 1);
    let event = &trace.events[0];
    assert_eq!(event["event"], "im:read:result");
    assert_eq!(event["data"]["req_id"], "g10d-locate-request");
    let body = &event["data"]["body"];
    assert_eq!(body["reqId"], "g10d-locate-request");
    assert_eq!(body["channelId"], "chfixx0000000000000000001c");
    assert_eq!(body["operation"], "locate");
    assert_eq!(body["targetPostId"], "srvfix0000000000000000000a");
    assert_eq!(body["targetIndex"], 5);
    assert_eq!(body["hasOlder"], true);
    assert_eq!(body["hasNewer"], true);
    assert_eq!(
        body["messages"]
            .as_array()
            .ok_or_else(|| "G-11 messages must be array".to_string())?
            .len(),
        11
    );
    assert_eq!(body["messages"][5]["id"], "srvfix0000000000000000000a");
    Ok(())
}

/// G-11 scroll intent 只表达目标 identity，不携带 renderer directive。
#[test]
fn g11_keeps_scroll_execution_in_the_consumer() -> TestResult<()> {
    let trace = run_success()?;
    let encoded = serde_json::to_string(&trace.events).map_err(|error| error.to_string())?;
    for forbidden in [
        "InteractionFrame",
        "Surface",
        "ActivateWindow",
        "Scroll",
        "directive",
        "mutation",
        "actionCapabilities",
    ] {
        assert!(!encoded.contains(forbidden), "forbidden field {forbidden}");
    }
    Ok(())
}

/// Rust producer 在生成模式下写出 G-11 sender 与空 receiver canonical artifact。
#[test]
fn g11_writes_reviewable_persona_artifacts() -> TestResult<()> {
    let trace = run_success()?;
    let Some(output) = std::env::var_os("HELIX_GATE_ARTIFACT_DIR") else {
        return Ok(());
    };
    let bundle = artifact::gate_bundle(
        "G-11",
        "message_v3_g11",
        trace.ticks,
        trace.events,
        Vec::new(),
        json!([
            {"table": "post", "columns": ["id", "temporary_id", "channel_id", "user_id", "type", "message", "create_at"]}
        ]),
        json!([
            {"table": "message", "columns": ["id", "temporary_id", "channel_id", "user_id", "type", "message", "send_status", "create_at", "event_seq"]}
        ]),
        json!([
            {"table": "message", "columns": ["id", "temporary_id", "channel_id", "user_id", "type", "message", "send_status", "create_at", "event_seq"]}
        ]),
    );
    artifact::write_bundle(std::path::Path::new(&output), &bundle)
        .map_err(|error| error.to_string())
        .map(|_| ())
}
