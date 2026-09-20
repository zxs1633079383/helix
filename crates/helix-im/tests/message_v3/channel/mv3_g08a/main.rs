use bytes::Bytes;
use helix_core::effect::Effect;
use helix_core::tick::InboundBytes;
use helix_core::Tick;
use serde_json::{json, Value};

#[path = "../../../common.rs"]
mod common;
#[path = "../../support/phase1_reference.rs"]
mod phase1_reference;

use common::{
    command_json, enveloped, port_reply_err, port_reply_ok, port_reply_with_body, EffectKind,
    Harness,
};

const ACTOR_ID: &str = "user-actor-444";
const RECEIVER_A_ID: &str = "user-receiver-445";
const RECEIVER_B_ID: &str = "user-receiver-446";
const OWNER_TEAM_ID: &str = "durable-owner-company";
const RECEIVER_A_TEAM_ID: &str = "durable-receiver-a-company";
const RECEIVER_B_TEAM_ID: &str = "durable-receiver-b-company";
const NOW_MS: u64 = 1_700_008_000_000;

/// 构造 G08a 的最小建群意图，身份信息不由客户端重复提交。
fn create_command() -> Tick {
    command_json(
        "im_create_channel",
        json!({
            "type": "P",
            "user_ids": [RECEIVER_A_ID, RECEIVER_B_ID],
            "display_name": "Phase 1 验收群",
            "force_create": false,
            "orient": "持续交付",
            "req_id": "req-mv3-g08a-001"
        }),
    )
}

/// 构造 Go channel/create 与 channel_created 共用的权威频道对象。
fn channel_authority() -> Value {
    json!({
        "id": common::cid(808),
        "type": "P",
        "userId": ACTOR_ID,
        "displayName": "Phase 1 验收群",
        "orient": "持续交付",
        "mentionPermission": "MANAGER",
        "noticePermission": "MANAGER",
        "topPermission": "MANAGER",
        "teamId": "company-phase1",
        "createAt": NOW_MS,
        "pictureType": "USER",
        "picture": {"userIds": [ACTOR_ID]},
        "owner": {
            "id": ACTOR_ID,
            "role": "OWNER",
            "nickName": "Actor 444",
            "teamId": OWNER_TEAM_ID
        },
        "members": [
            {
                "id": RECEIVER_A_ID,
                "role": "MEMBER",
                "nickName": "Receiver 445",
                "teamId": RECEIVER_A_TEAM_ID
            },
            {
                "id": RECEIVER_B_ID,
                "role": "MEMBER",
                "nickName": "Receiver 446",
                "teamId": RECEIVER_B_TEAM_ID
            }
        ]
    })
}

/// 从当前 effect batch 中读取指定类别的 correlation。
fn correlation_of(effects: &[Effect], kind: &str) -> u64 {
    effects
        .iter()
        .find_map(|effect| match (kind, effect) {
            ("http", Effect::Http { corr, .. })
            | ("persist", Effect::PersistAtomic { corr, .. }) => Some(corr.raw()),
            _ => None,
        })
        .unwrap_or_else(|| panic!("missing {kind} correlation"))
}

/// 从当前 effect batch 解码指定名称的 MessageV3 投影。
fn emitted_event(effects: &[Effect], event_name: &str) -> Value {
    effects
        .iter()
        .filter_map(|effect| {
            let Effect::Emit { event } = effect else {
                return None;
            };
            serde_json::from_slice::<Value>(event.0.as_ref()).ok()
        })
        .find(|event| event["event"] == event_name)
        .unwrap_or_else(|| panic!("missing {event_name} projection"))
}

/// 将权威频道包装成现网 channel_created WS action。
fn channel_created_inbound() -> Tick {
    let frame = json!({
        "action": "channel_created",
        "data": channel_authority()
    });
    Tick::Inbound(InboundBytes(Bytes::from(
        serde_json::to_vec(&frame).expect("channel_created frame must serialize"),
    )))
}

/// 断言提交后的 created 投影完全符合 Angular 直接 Upsert 的绝对态与头像字段契约。
fn assert_created_projection(event: &Value, viewer_role: &str) {
    let mut data = event["data"].clone();
    data.as_object_mut()
        .expect("created data must be an object")
        .remove("tracing");
    assert_eq!(
        data,
        json!({
            "id": common::cid(808),
            "channelId": common::cid(808),
            "type": "P",
            "userId": ACTOR_ID,
            "displayName": "Phase 1 验收群",
            "orient": "持续交付",
            "mentionPermission": "MANAGER",
            "noticePermission": "MANAGER",
            "topPermission": "MANAGER",
            "createAt": NOW_MS,
            "pictureType": "USER",
            "picture": {"userIds": [ACTOR_ID]},
            "memberIds": [ACTOR_ID, RECEIVER_A_ID, RECEIVER_B_ID],
            "memberCount": 3,
            "members": [
                {"userId": ACTOR_ID, "teamId": OWNER_TEAM_ID, "role": "OWNER", "nickName": "Actor 444"},
                {"userId": RECEIVER_A_ID, "teamId": RECEIVER_A_TEAM_ID, "role": "MEMBER", "nickName": "Receiver 445"},
                {"userId": RECEIVER_B_ID, "teamId": RECEIVER_B_TEAM_ID, "role": "MEMBER", "nickName": "Receiver 446"}
            ],
            "owner": {"userId": ACTOR_ID, "teamId": OWNER_TEAM_ID, "role": "OWNER", "nickName": "Actor 444"},
            "ownerId": ACTOR_ID,
            "viewerRole": viewer_role,
            "unreadCount": 0,
            "mentionCount": 0
        })
    );
}

/// 断言 members terminal 也只传播持久化成员身份，不从当前 company 补齐 teamId。
fn assert_members_projection(event: &Value) {
    assert_eq!(
        event["data"],
        json!({
            "channelId": common::cid(808),
            "members": [
                {"userId": ACTOR_ID, "teamId": OWNER_TEAM_ID, "role": "OWNER", "nickName": "Actor 444"},
                {"userId": RECEIVER_A_ID, "teamId": RECEIVER_A_TEAM_ID, "role": "MEMBER", "nickName": "Receiver 445"},
                {"userId": RECEIVER_B_ID, "teamId": RECEIVER_B_TEAM_ID, "role": "MEMBER", "nickName": "Receiver 446"}
            ],
            "leaves": []
        })
    );
}

/// 证明 MV3-G08a 的 visual 目标参考包完整且未伪装为运行时证据。
#[test]
fn reference_package_is_complete() {
    phase1_reference::assert_reference_gate("MV3-G08a", "channel/mv3-g08a");
}

/// 发送者建群必须依次经过真实 HTTP、原子持久化，再发布唯一终态投影。
#[test]
fn sender_create_is_http_then_persist_then_absolute_projection() {
    let mut harness = Harness::new_with_identity(&[], ACTOR_ID, "company-phase1", "", "", "");

    let command_effects = harness.step(create_command(), NOW_MS);
    assert_eq!(command_effects.len(), 1);
    let http_corr = correlation_of(harness.sink.as_slice(), "http");
    let Effect::Http { req, .. } = &harness.sink.as_slice()[0] else {
        panic!("im_create_channel must start with HTTP");
    };
    assert!(req.url.ends_with("/channel/create"));
    assert_eq!(
        serde_json::from_slice::<Value>(
            req.body
                .as_ref()
                .expect("channel/create body is required")
                .as_ref()
        )
        .expect("channel/create body must be json"),
        json!({
            "type": "P",
            "userIds": [RECEIVER_A_ID, RECEIVER_B_ID],
            "displayName": "Phase 1 验收群",
            "forceCreate": false,
            "orient": "持续交付"
        })
    );

    let response = json!({"status": "SUCCESS", "data": channel_authority()});
    let http_reply_effects = harness.step(
        port_reply_with_body(
            http_corr,
            enveloped(Bytes::from(
                serde_json::to_vec(&response).expect("response must serialize"),
            )),
        ),
        NOW_MS + 1,
    );
    assert_eq!(http_reply_effects.len(), 1);
    assert!(matches!(
        http_reply_effects.as_slice(),
        [EffectKind::PersistAtomic(_)]
    ));
    assert!(!harness
        .sink
        .as_slice()
        .iter()
        .any(|effect| matches!(effect, Effect::Emit { .. })));

    let persist_corr = harness
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::PersistAtomic { corr, .. } => Some(corr.raw()),
            _ => None,
        })
        .unwrap_or_else(|| panic!("missing persist correlation: {:?}", harness.sink.as_slice()));
    harness.step(port_reply_ok(persist_corr), NOW_MS + 2);
    let created = emitted_event(harness.sink.as_slice(), "im:channel:created");
    assert_created_projection(&created, "OWNER");
    assert_members_projection(&emitted_event(
        harness.sink.as_slice(),
        "im:channel:members",
    ));
    assert_eq!(created["data"]["tracing"]["requestId"], "req-mv3-g08a-001");
}

/// 接收者消费同一 WS authority 时保持成员顺序，且重复帧不重复发布。
#[test]
fn receiver_ws_projection_is_ordered_and_idempotent() {
    let mut harness = Harness::new_with_identity(&[], RECEIVER_A_ID, "company-phase1", "", "", "");

    let initial = harness.step(channel_created_inbound(), NOW_MS);
    assert!(matches!(initial.as_slice(), [EffectKind::PersistAtomic(_)]));
    let persist_corr = correlation_of(harness.sink.as_slice(), "persist");
    harness.step(port_reply_ok(persist_corr), NOW_MS + 1);
    assert_created_projection(
        &emitted_event(harness.sink.as_slice(), "im:channel:created"),
        "MEMBER",
    );
    assert_members_projection(&emitted_event(
        harness.sink.as_slice(),
        "im:channel:members",
    ));

    let duplicate = harness.step(channel_created_inbound(), NOW_MS + 2);
    assert!(
        duplicate.is_empty(),
        "committed channel_created must be idempotent"
    );
}

/// 新频道 WS 回声若先于 HTTP 回包到达，仍必须保留服务端 envelope 的关联 ID。
#[test]
fn receiver_ws_projection_carries_server_track_id() {
    let frame = json!({
        "action": "channel_created",
        "data": channel_authority(),
        "tracing": {"csesTrackId": "req-ws-channel-create"}
    });
    let mut harness = Harness::new_with_identity(&[], RECEIVER_A_ID, "company-phase1", "", "", "");
    harness.step(
        Tick::Inbound(InboundBytes(Bytes::from(
            serde_json::to_vec(&frame).expect("channel_created frame must serialize"),
        ))),
        NOW_MS,
    );
    let persist_corr = correlation_of(harness.sink.as_slice(), "persist");
    harness.step(port_reply_ok(persist_corr), NOW_MS + 1);

    let created = emitted_event(harness.sink.as_slice(), "im:channel:created");
    assert_eq!(
        created["data"]["tracing"]["requestId"],
        "req-ws-channel-create"
    );
}

/// 持久化失败时禁止发布 created/members，避免 Angular 看见不可恢复的假终态。
#[test]
fn persist_failure_suppresses_terminal_projection() {
    let mut harness = Harness::new_with_identity(&[], ACTOR_ID, "company-phase1", "", "", "");
    let command_effects = harness.step(create_command(), NOW_MS);
    let http_corr = match command_effects.as_slice() {
        [EffectKind::Http(corr)] => *corr,
        other => panic!("expected one channel/create HTTP, got {other:?}"),
    };
    let response = json!({"status": "SUCCESS", "data": channel_authority()});
    harness.step(
        port_reply_with_body(
            http_corr,
            enveloped(Bytes::from(
                serde_json::to_vec(&response).expect("response must serialize"),
            )),
        ),
        NOW_MS + 1,
    );
    let persist_corr = correlation_of(harness.sink.as_slice(), "persist");

    let failure_effects = harness.step(port_reply_err(persist_corr), NOW_MS + 2);
    assert!(failure_effects.is_empty());
    assert!(!harness
        .sink
        .as_slice()
        .iter()
        .any(|effect| matches!(effect, Effect::Emit { .. })));
}

/// 配额拒绝必须保留请求关联和Go文案，重复回包不能重复提示或产生持久化。
#[test]
fn quota_rejection_emits_correlated_failure_once() {
    let mut harness = Harness::new_with_identity(&[], ACTOR_ID, "company-phase1", "", "", "");
    harness.step(create_command(), NOW_MS);
    let corr = correlation_of(harness.sink.as_slice(), "http");
    let body=enveloped(Bytes::from(serde_json::to_vec(&json!({"status":"failed","message":"同一用户在每个公司最多只能创建 200 个未关闭的群聊"})).unwrap()));
    harness.step(port_reply_with_body(corr, body.clone()), NOW_MS + 1);
    let failure = emitted_event(harness.sink.as_slice(), "im:channel:create-failed");
    assert_eq!(
        failure["data"],
        json!({"requestId":"req-mv3-g08a-001","reason":"business_rejected","message":"同一用户在每个公司最多只能创建 200 个未关闭的群聊"})
    );
    assert!(harness
        .sink
        .as_slice()
        .iter()
        .all(|effect| matches!(effect, Effect::Emit { .. })));
    assert_eq!(harness.sink.as_slice().len(), 1);
    assert!(harness
        .step(port_reply_with_body(corr, body), NOW_MS + 2)
        .is_empty());
}

/// 端口失败与畸形HTTP回包必须有可消费失败终态，不能污染成功持久化链。
#[test]
fn create_transport_and_invalid_reply_fail_closed_once() {
    for transport in [true, false] {
        let mut harness = Harness::new_with_identity(&[], ACTOR_ID, "company-phase1", "", "", "");
        harness.step(create_command(), NOW_MS);
        let corr = correlation_of(harness.sink.as_slice(), "http");
        let reply = if transport {
            port_reply_err(corr)
        } else {
            port_reply_with_body(corr, enveloped(Bytes::from_static(b"{invalid")))
        };
        harness.step(reply, NOW_MS + 1);
        let failure = emitted_event(harness.sink.as_slice(), "im:channel:create-failed");
        assert_eq!(
            failure["data"]["reason"],
            if transport {
                "transport_failed"
            } else {
                "invalid_response"
            }
        );
        assert!(harness
            .sink
            .as_slice()
            .iter()
            .all(|effect| matches!(effect, Effect::Emit { .. })));
        assert!(harness.step(port_reply_err(corr), NOW_MS + 2).is_empty());
    }
}
