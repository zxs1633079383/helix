//! 统一失败来源合同：公开 Command/Inbound → 真实 correlation 回灌 → 错误终态。
//! 远程读失败不持久化；成功透传不变；重复/迟到回复不得重复终态。
//! 这里只证明 sans-IO 回放，不代表 Native/Go/UI 验收。
mod common;
use common::*;
use helix_core::tick::{AppCommand, InboundBytes};
use helix_core::{Effect, Module, Tick};
use serde_json::{json, Value};

/// 从公开入口发起请求并取得实际 HTTP correlation。
fn request(h: &mut Harness, command: &'static str, req_id: &str) -> u64 {
    h.step(
        Tick::Command(AppCommand::new(
            command,
            serde_json::to_vec(&json!({
                "channel_id":"ch00000000000000000000000a", "req_id":req_id
            }))
            .unwrap(),
        )),
        1000,
    );
    h.sink
        .as_slice()
        .iter()
        .find_map(|e| match e {
            Effect::Http { corr, .. } => Some(corr.raw()),
            _ => None,
        })
        .expect("command must issue HTTP")
}

/// 读取公开错误终态，而不是测试自行构造事件。
fn error(h: &Harness) -> String {
    h.sink
        .as_slice()
        .iter()
        .find_map(|e| match e {
            Effect::Emit { event } => serde_json::from_slice::<Value>(&event.0)
                .ok()
                .and_then(|v| v["data"]["error"].as_str().map(str::to_owned)),
            _ => None,
        })
        .expect("must emit error terminal")
}

/// 回放现场 HTTP200/failed，不得再用 missing data 覆盖真正的成员查询错误。
#[test]
fn hydration_business_failure_has_command_http_path_and_original_reason() {
    let mut h = Harness::new_with_identity(&[], "viewer", "company", "", "", "");
    h.seed_channel_scope("ch00000000000000000000000a", "company", "viewer");
    let corr = request(
        &mut h,
        "im_channel_load_increment_by_channel_id",
        "hydration-failure",
    );
    h.step(
        port_reply_with_body(
            corr,
            enveloped(bytes::Bytes::from_static(
                br#"{"status":"failed","message":"ent: channel_member not found"}"#,
            )),
        ),
        1001,
    );
    let message = error(&h);
    for expected in [
        "im_channel_load_increment_by_channel_id",
        "/channel/load/incrementByChannelId",
        "hydration-failure",
        "failed",
        "ent: channel_member not found",
    ] {
        assert!(message.contains(expected), "missing {expected}: {message}");
    }
    assert!(!message.contains("missing data"), "{message}");
    h.step(
        port_reply_with_body(corr, enveloped(bytes::Bytes::from_static(br#"{}"#))),
        1002,
    );
    assert!(
        !h.sink
            .as_slice()
            .iter()
            .any(|e| matches!(e, Effect::Emit { .. })),
        "late reply must not emit"
    );
}

/// 普通 read relay 必须使用同一个失败处理入口，不能只修 hydration。
#[test]
fn ordinary_read_business_failure_is_error_with_its_own_origin() {
    let mut h = Harness::new_with_identity(&[], "viewer", "company", "", "", "");
    h.seed_channel_scope("ch00000000000000000000000a", "company", "viewer");
    let corr = request(&mut h, "im_top20", "read-failure");
    h.step(
        port_reply_with_body(
            corr,
            enveloped(bytes::Bytes::from_static(
                br#"{"status":"failed","message":"access denied"}"#,
            )),
        ),
        1001,
    );
    let message = error(&h);
    for expected in ["im_top20", "read-failure", "access denied", "HTTP"] {
        assert!(message.contains(expected), "missing {expected}: {message}");
    }
}

/// 并发回复乱序时仍须绑定各自 Command 与请求，禁止串到最后执行的命令。
#[test]
fn interleaved_http_failures_keep_separate_origins() {
    let mut h = Harness::new_with_identity(&[], "viewer", "company", "", "", "");
    h.seed_channel_scope("ch00000000000000000000000a", "company", "viewer");
    let a = request(&mut h, "im_top20", "request-a");
    let b = request(
        &mut h,
        "im_channel_load_increment_by_channel_id",
        "request-b",
    );
    for (corr, id, cmd, other) in [
        (
            b,
            "request-b",
            "im_channel_load_increment_by_channel_id",
            "request-a",
        ),
        (a, "request-a", "im_top20", "request-b"),
    ] {
        h.step(
            port_reply_with_body(
                corr,
                enveloped(bytes::Bytes::from_static(
                    br#"{"status":"failed","message":"access denied"}"#,
                )),
            ),
            1001,
        );
        let message = error(&h);
        assert!(message.contains(id) && message.contains(cmd), "{message}");
        assert!(!message.contains(other), "{message}");
    }
}

/// 服务器主动推送没有可证明的 Command，须显示 WS action 与 server_push。
#[test]
fn websocket_handler_error_names_action_without_inventing_command() {
    let mut h = Harness::new_with_identity(&[], "viewer", "company", "", "", "");
    h.seed_channel_scope("ch00000000000000000000000a", "company", "viewer");
    let err = h
        .module
        .handle(
            &Tick::Inbound(InboundBytes::from_static(
                br#"{"action":"unknown_test_action","data":{}}"#,
            )),
            1000,
            &mut h.sink,
        )
        .unwrap_err();
    let message = err.to_string();
    assert!(
        message.contains("WS")
            && message.contains("unknown_test_action")
            && message.contains("server_push"),
        "{message}"
    );
}

/// HTTP 状态码、畸形响应与 driver 分类错误均保留同一个来源。
#[test]
fn http_and_transport_failures_keep_origin_and_status() {
    use helix_core::tick::{PortError, PortOutcome};
    for (outcome, expected) in [
        (PortOutcome::Err(PortError::Timeout), "Timeout"),
        (PortOutcome::Err(PortError::Network), "Network"),
        (PortOutcome::Err(PortError::Http(503)), "503"),
    ] {
        let mut h = Harness::new_with_identity(&[], "viewer", "company", "", "", "");
        h.seed_channel_scope("ch00000000000000000000000a", "company", "viewer");
        let corr = request(&mut h, "im_top20", "transport-test");
        h.step(
            Tick::PortReply {
                corr: helix_core::Correlation::from_raw(corr),
                outcome,
            },
            1001,
        );
        let message = error(&h);
        for part in [expected, "im_top20", "transport-test", "HTTP"] {
            assert!(message.contains(part), "{message}");
        }
    }
}

/// 已成功的 raw body 完全保留，不附加诊断字段或修改业务数据。
#[test]
fn successful_read_body_remains_unchanged() {
    let mut h = Harness::new_with_identity(&[], "viewer", "company", "", "", "");
    h.seed_channel_scope("ch00000000000000000000000a", "company", "viewer");
    let corr = request(&mut h, "im_top20", "success-test");
    let body = json!({"status":"SUCCESS","data":[{"id":"p1","message":"original"}]});
    h.step(
        port_reply_with_body(corr, enveloped(serde_json::to_vec(&body).unwrap().into())),
        1001,
    );
    let value = h
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Emit { event } => serde_json::from_slice::<Value>(&event.0).ok(),
            _ => None,
        })
        .unwrap();
    assert_eq!(
        value,
        json!({"event":"im:read:result","data":{"req_id":"success-test","body":body}})
    );
}

/// failed 即使带有似成功 data，也不能继续持久化或发布成功。
#[test]
fn failed_envelope_with_data_cannot_become_success() {
    let mut h = Harness::new_with_identity(&[], "viewer", "company", "", "", "");
    h.seed_channel_scope("ch00000000000000000000000a", "company", "viewer");
    let corr = request(&mut h, "im_top20", "failed-data");
    h.step(
        port_reply_with_body(
            corr,
            enveloped(bytes::Bytes::from_static(
                br#"{"status":"failed","message":null,"data":[{"id":"p1"}]}"#,
            )),
        ),
        1001,
    );
    assert!(error(&h).contains("remote request rejected"));
    assert!(!h
        .sink
        .as_slice()
        .iter()
        .any(|e| matches!(e, Effect::Persist { .. } | Effect::PersistAtomic { .. })));
}

/// 后续 PersistErr 保留最初的命令和 HTTP 来源，不能退化为无来源的存储错误。
#[test]
fn followup_persist_error_keeps_original_http_and_command() {
    use helix_core::tick::{PortError, PortOutcome};
    let mut h = Harness::new_with_identity(&[], "viewer", "company", "", "", "");
    h.seed_channel_scope("ch00000000000000000000000a", "company", "viewer");
    let corr = request(
        &mut h,
        "im_channel_load_increment_by_channel_id",
        "persist-error",
    );
    h.step(
        port_reply_with_body(
            corr,
            enveloped(bytes::Bytes::from_static(
                br#"{"status":"SUCCESS","data":{"id":"ch00000000000000000000000a","teamId":"company","displayName":"closed","deleteAt":1789897616720,"lastEventSeq":7,"unreadCount":0,"memberCount":1,"members":[{"userId":"viewer","role":"MEMBER"}],"type":"P"}}"#,
            )),
        ),
        1001,
    );
    let persist = h
        .sink
        .as_slice()
        .iter()
        .find_map(|e| match e {
            Effect::Persist { corr, .. } | Effect::PersistAtomic { corr, .. } => Some(*corr),
            _ => None,
        })
        .expect("authority snapshot needs persist");
    h.step(
        Tick::PortReply {
            corr: persist,
            outcome: PortOutcome::Err(PortError::Storage(9)),
        },
        1002,
    );
    let message = error(&h);
    for part in [
        "im_channel_load_increment_by_channel_id",
        "/channel/load/incrementByChannelId",
        "persist-error",
        "Storage(9)",
    ] {
        assert!(message.contains(part), "{message}");
    }
}

/// HTTP admission 后由 WS 持久化失败终结；机器码保持不变，上下文仍能追溯原命令。
#[test]
fn ws_persist_failure_keeps_machine_code_and_origin() {
    let mut h = Harness::new_with_identity(&[1], "actor", "company", "", "", "");
    h.step(
        Tick::Command(AppCommand::new(
            "im_create_schedule",
            serde_json::to_vec(&json!({
                "channel_id":cid(1),"message":"scheduled","type":"TEXT",
                "schedule_post_at":1900000000000i64,"req_id":"schedule-context"
            }))
            .unwrap(),
        )),
        1,
    );
    let http = h
        .sink
        .as_slice()
        .iter()
        .find_map(|e| match e {
            Effect::Http { corr, .. } => Some(corr.raw()),
            _ => None,
        })
        .unwrap();
    h.step(
        port_reply_with_body(
            http,
            enveloped(bytes::Bytes::from_static(br#"{"status":"SUCCESS"}"#)),
        ),
        2,
    );
    h.step(
        Tick::Inbound(InboundBytes(
            serde_json::to_vec(&json!({
                "action":"post_schedule_created","data":{
                    "channelId":cid(1),"scheduleId":"s1","scheduleOwnerUserId":"actor",
                    "message":"scheduled","sendAt":1900000000000i64,"revision":8
                }
            }))
            .unwrap()
            .into(),
        )),
        3,
    );
    let persist = h
        .sink
        .as_slice()
        .iter()
        .find_map(|e| match e {
            Effect::PersistAtomic { corr, .. } => Some(corr.raw()),
            _ => None,
        })
        .unwrap();
    h.step(port_reply_err(persist), 4);
    let event = h
        .sink
        .as_slice()
        .iter()
        .find_map(|e| match e {
            Effect::Emit { event } => serde_json::from_slice::<Value>(&event.0).ok(),
            _ => None,
        })
        .unwrap();
    assert_eq!(event["data"]["error"], "SCHEDULE_PERSIST_FAILED");
    let context = event["data"]["errorContext"].as_str().unwrap();
    for part in [
        "im_create_schedule",
        "WS post_schedule_created",
        "schedule-context",
    ] {
        assert!(context.contains(part), "{context}");
    }
}

/// 非 2xx 的 HTTP 状态与服务端业务原因必须同时可见。
#[test]
fn non_2xx_preserves_business_cause_together_with_http_status() {
    let mut h = Harness::new_with_identity(&[], "viewer", "company", "", "", "");
    h.seed_channel_scope("ch00000000000000000000000a", "company", "viewer");
    let corr = request(&mut h, "im_top20", "non-2xx");
    let mut reply: Value = serde_json::from_slice(&enveloped(bytes::Bytes::from_static(
        br#"{"status":"failed","message":"membership required"}"#,
    )))
    .unwrap();
    reply["status"] = json!(403);
    h.step(
        port_reply_with_body(corr, serde_json::to_vec(&reply).unwrap().into()),
        1001,
    );
    let message = error(&h);
    for part in ["403", "membership required", "im_top20", "HTTP"] {
        assert!(message.contains(part), "{message}");
    }
}
