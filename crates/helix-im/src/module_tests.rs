//! `module.rs` 单测外提 sibling（structure-gate「测试外提」范本，对齐 `state_tests.rs`）。
//! A4 hello 握手回放测试（直接断言 `pub(crate)` state）。`#[path]` 子模块：`super` = `module`。

use super::*;
use bytes::Bytes;
use helix_core::effect::{Correlation, TransportId};
use helix_core::tick::{InboundBytes, PortOutcome, ReplyBytes};
use helix_core::Effect;

fn module_with_channel() -> ImModule {
    let mut m = ImModule::new(ImConfig::default());
    m.register_channel(crate::state::test_channel_id(1), 0);
    m
}

fn hello_frame(conn_id: &str) -> Tick {
    let body = serde_json::json!({
        "action": "hello",
        "data": { "connectionId": conn_id, "serverVersion": "x", "pingInterval": 30000u64 }
    });
    Tick::Inbound(InboundBytes(serde_json::to_vec(&body).unwrap().into()))
}

fn has_http(sink: &EffectSink) -> bool {
    sink.as_slice()
        .iter()
        .any(|e| matches!(e, Effect::Http { .. }))
}

#[test]
fn online_post_requeries_an_attached_timeline_after_its_durable_write() {
    let channel_id = crate::state::test_channel_id(31);
    let mut module = ImModule::new(ImConfig::default());
    module.register_channel(channel_id, 0);
    let mut sink = EffectSink::new();
    let surface_request = crate::surface_runtime::SurfaceResyncRequest {
        surface: crate::interaction_frame::SurfaceId::MessageTimeline,
        scope: crate::interaction_frame::SurfaceScope::Timeline {
            channel_id: channel_id.as_str().to_string(),
            window_token: "latest".to_string(),
        },
        causation_id: None,
    };
    module
        .handle_surface_resync(
            &serde_json::to_vec(&surface_request).expect("surface request serializes"),
            1,
            &mut sink,
        )
        .expect("timeline attaches before online post");
    sink.clear();

    let post = serde_json::json!({
        "action": "post",
        "data": {
            "channelId": channel_id.as_str(),
            "event_seq": 1,
            "post": {
                "id": "message-online-1",
                "channelId": channel_id.as_str(),
                "userId": "user-online-1",
                "createAt": 1_000,
                "type": "TEXT",
                "message": "online frame fact",
                "props": { "channel_event_seq": 1 }
            }
        }
    });
    module
        .handle(
            &Tick::Inbound(InboundBytes(serde_json::to_vec(&post).unwrap().into())),
            1_000,
            &mut sink,
        )
        .expect("online post dispatches");

    let message_write_index = sink
        .as_slice()
        .iter()
        .position(|effect| {
            matches!(effect, Effect::PersistFire { ops } if ops.iter().any(|op| match op {
                helix_core::effect::StorageOp::BatchUpsert(spec) => spec.table == "message",
                _ => false,
            }))
        })
        .expect("accepted online post persists a message fact");
    let readback_index = sink
        .as_slice()
        .iter()
        .position(|effect| matches!(effect, Effect::Persist { ops, .. } if matches!(ops.as_slice(), [helix_core::effect::StorageOp::Scan(spec)] if spec.table == "message")))
        .unwrap_or_else(|| panic!("attached timeline receives a local-first readback: {:?}", sink.as_slice()));

    assert!(
        message_write_index < readback_index,
        "the typed Frame readback must be queued after the WS message write"
    );
}

#[test]
fn legacy_post_requeries_an_attached_timeline_after_its_durable_write() {
    let channel_id = crate::state::test_channel_id(32);
    let mut module = ImModule::new(ImConfig::default());
    module.register_channel(channel_id, 0);
    let mut sink = EffectSink::new();
    let surface_request = crate::surface_runtime::SurfaceResyncRequest {
        surface: crate::interaction_frame::SurfaceId::MessageTimeline,
        scope: crate::interaction_frame::SurfaceScope::Timeline {
            channel_id: channel_id.as_str().to_string(),
            window_token: "latest".to_string(),
        },
        causation_id: None,
    };
    module
        .handle_surface_resync(
            &serde_json::to_vec(&surface_request).expect("surface request serializes"),
            1,
            &mut sink,
        )
        .expect("timeline attaches before legacy post");
    sink.clear();

    let post = serde_json::json!({
        "event": "posted",
        "data": {
            "channel_id": channel_id.as_str(),
            "seq": 1,
            "event_type": 1,
            "msg_id": "message-legacy-1",
            "actor_id": "user-legacy-1",
            "created_at": 1_000,
            "payload": null
        }
    });
    module
        .handle(
            &Tick::Inbound(InboundBytes(serde_json::to_vec(&post).unwrap().into())),
            1_000,
            &mut sink,
        )
        .expect("legacy post dispatches");

    assert!(
        sink.as_slice().iter().any(|effect| matches!(effect, Effect::Persist { ops, .. } if matches!(ops.as_slice(), [helix_core::effect::StorageOp::Scan(spec)] if spec.table == "message"))),
        "attached legacy post also schedules a local-first timeline readback"
    );
}

#[test]
fn runtime_identity_enables_member_projection_without_wire_user_id() {
    let mut m = ImModule::new(ImConfig::default());
    let mut sink = EffectSink::new();
    let identity = serde_json::json!({
        "auth_user_id": "user-runtime",
        "company_id": "company-runtime"
    });

    assert!(m.accepts(&Tick::Command(helix_core::tick::AppCommand::new(
        RUNTIME_IDENTITY_COMMAND,
        serde_json::to_vec(&identity).unwrap(),
    ))));
    m.handle(
        &Tick::Command(helix_core::tick::AppCommand::new(
            RUNTIME_IDENTITY_COMMAND,
            serde_json::to_vec(&identity).unwrap(),
        )),
        1_000,
        &mut sink,
    )
    .unwrap();
    assert_eq!(m.config.auth_user_id, "user-runtime");
    assert_eq!(m.config.company_id, "company-runtime");
    assert!(sink.is_empty(), "身份同步不得产生业务副作用");

    let update = serde_json::json!({
        "action": "update_channel",
        "data": {
            "id": crate::state::test_channel_id(13).as_str(),
            "unreadCount": 0,
            "mentionCount": 0,
            "updateAt": 1_001
        }
    });
    m.handle(
        &Tick::Inbound(InboundBytes(serde_json::to_vec(&update).unwrap().into())),
        1_001,
        &mut sink,
    )
    .unwrap();

    assert!(
        sink.as_slice().iter().any(|effect| match effect {
            Effect::Emit { event } => {
                let envelope: serde_json::Value = serde_json::from_slice(event.0.as_ref()).unwrap();
                envelope["event"] == "im:channel:update"
            }
            _ => false,
        }),
        "member 绝对态写入后仍须发布 channel surface 通知"
    );
}

#[test]
fn runtime_identity_after_hello_reopens_recovery_for_the_trusted_actor() {
    let mut m = ImModule::new(ImConfig::default());
    m.state.conn = crate::state::ConnState::Connected;
    m.state.connection_id = Some("connection-b".to_string());
    m.state.recovery_session.begin("bootstrap");
    let mut sink = EffectSink::new();
    let identity = serde_json::json!({
        "auth_user_id": "viewer-b",
        "company_id": "team-b"
    });

    m.handle(
        &Tick::Command(helix_core::tick::AppCommand::new(
            RUNTIME_IDENTITY_COMMAND,
            serde_json::to_vec(&identity).unwrap(),
        )),
        2_000,
        &mut sink,
    )
    .unwrap();

    assert!(m.state.recovery_session.is_collecting_for("viewer-b"));
    assert_eq!(m.state.recovery_session.actor_id, "viewer-b");
    assert!(
        sink.is_empty(),
        "重绑恢复会话只更新确定性内核状态，不直接发布 Frame"
    );
}

#[test]
fn runtime_identity_change_invalidates_old_surface_slots_and_query_correlations() {
    let channel_id = crate::state::test_channel_id(21);
    let mut m = ImModule::new(ImConfig {
        auth_user_id: "viewer-a".to_string(),
        company_id: "team-a".to_string(),
        ..ImConfig::default()
    });
    let mut sink = EffectSink::new();
    let dialog_corr = Correlation::from_raw(8_001);
    let message_corr = Correlation::from_raw(8_002);
    let retained_corr = Correlation::from_raw(8_003);
    let surface_corr = Correlation::from_raw(8_004);

    m.state
        .dialog_list_frame
        .snapshot(
            crate::dialog_frame::DialogListScope::new("viewer-a", "team-a"),
            Vec::new(),
        )
        .expect("old viewer dialog slot attaches");
    let timeline_scope = crate::interaction_frame::SurfaceScope::Timeline {
        channel_id: channel_id.as_str().to_string(),
        window_token: "latest".to_string(),
    };
    m.state
        .timeline_projector
        .snapshot_from_render_ready(
            crate::projector::timeline::TimelineWindowRequest::latest(channel_id.as_str()),
            &[serde_json::json!({
                "msgId": "message-a",
                "channelId": channel_id.as_str(),
                "userId": "viewer-a",
                "createAt": 1,
                "text": "old viewer",
                "type": "TEXT",
                "props": {},
                "actionCapabilities": {},
            })],
        )
        .expect("old viewer timeline slot attaches");
    m.state
        .message_query_generations
        .insert((channel_id, "latest".to_string()), 1);
    m.state.corr_map.insert(
        dialog_corr,
        crate::state::CorrelationContext::DialogListQuery { causation_id: None },
    );
    m.state.corr_map.insert(
        message_corr,
        crate::state::CorrelationContext::MessageQueryLocal {
            request: Box::new(crate::query::MessageQueryRequest {
                channel_id,
                limit: 20,
                window_token: "latest".to_string(),
            }),
            query_session_epoch: m.state.query_session_epoch,
            query_generation: 1,
            allow_remote_fallback: true,
            causation_id: None,
            deferred_send_http: None,
        },
    );
    m.state.corr_map.insert(
        retained_corr,
        crate::state::CorrelationContext::OutboundReadReply {
            req_id: "read-stays-owned".to_string(),
        },
    );
    m.state.corr_map.insert(
        surface_corr,
        crate::state::CorrelationContext::SurfaceThreadSnapshot {
            request: crate::surface_runtime::SurfaceResyncRequest {
                surface: crate::interaction_frame::SurfaceId::MessageThreadDetail,
                scope: crate::interaction_frame::SurfaceScope::Thread {
                    channel_id: channel_id.as_str().to_string(),
                    root_message_id: "message-a".to_string(),
                },
                causation_id: None,
            },
            expected_revision: None,
        },
    );

    let identity = serde_json::json!({
        "auth_user_id": "viewer-b",
        "company_id": "team-b"
    });
    m.handle(
        &Tick::Command(helix_core::tick::AppCommand::new(
            RUNTIME_IDENTITY_COMMAND,
            serde_json::to_vec(&identity).unwrap(),
        )),
        2_000,
        &mut sink,
    )
    .unwrap();

    assert_eq!(m.state.query_session_epoch, 1);
    assert!(m.state.dialog_list_frame.scope().is_none());
    assert!(m
        .state
        .timeline_projector
        .current_view(&timeline_scope)
        .is_none());
    assert!(m.state.message_query_generations.is_empty());
    assert!(!m.state.corr_map.contains_key(&dialog_corr));
    assert!(!m.state.corr_map.contains_key(&message_corr));
    assert!(!m.state.corr_map.contains_key(&surface_corr));
    assert!(m.state.corr_map.contains_key(&retained_corr));

    // 回归锚 P1：A 的晚到 scan 已失去 corr，不得用 B 的运行时身份生成 frame。
    sink.clear();
    m.handle(
        &Tick::PortReply {
            corr: dialog_corr,
            outcome: PortOutcome::Ok(ReplyBytes(Bytes::from_static(b"[]"))),
        },
        2_100,
        &mut sink,
    )
    .unwrap();
    assert!(sink.is_empty());
}

/// Connected → conn 变 Connecting，且不产 resync Http（现网 Go 握手前 resync 会 401）。
#[test]
fn connected_sets_connecting_no_resync() {
    let mut m = module_with_channel();
    let mut sink = EffectSink::new();
    m.handle(&Tick::Connected(TransportId::from_raw(0)), 1000, &mut sink)
        .unwrap();
    assert_eq!(m.state.conn, ConnState::Connecting);
    assert!(!has_http(&sink), "握手前不得 resync");
}

/// hello Inbound → Connected + emit im:connection:established + resync Http + connectionId 存。
#[test]
fn hello_completes_handshake_and_resyncs() {
    let mut m = module_with_channel();
    let mut sink = EffectSink::new();
    m.handle(&Tick::Connected(TransportId::from_raw(0)), 1000, &mut sink)
        .unwrap();
    sink.clear();
    m.handle(&hello_frame("CONN_ABC"), 1001, &mut sink).unwrap();
    assert_eq!(m.state.conn, ConnState::Connected);
    assert_eq!(m.state.connection_id.as_deref(), Some("CONN_ABC"));
    assert!(
        sink.as_slice()
            .iter()
            .any(|e| matches!(e, Effect::Emit { .. })),
        "应 emit im:connection:established"
    );
    assert!(has_http(&sink), "hello 后应 resync");
}

/// hello 后构造的 sync Http headers 同时携带 connectionId 与 session auth intent。
#[test]
fn sync_http_carries_connection_id_header() {
    let mut m = module_with_channel();
    let mut sink = EffectSink::new();
    m.handle(&Tick::Connected(TransportId::from_raw(0)), 1000, &mut sink)
        .unwrap();
    sink.clear();
    m.handle(&hello_frame("CONN_XYZ"), 1001, &mut sink).unwrap();
    let req = sink
        .as_slice()
        .iter()
        .find_map(|e| match e {
            Effect::Http { req, .. } => Some(req),
            _ => None,
        })
        .expect("应有 resync Http");
    assert!(
        req.headers
            .iter()
            .any(|(k, v)| k == "connectionId" && v == "CONN_XYZ"),
        "sync Http 须带 connectionId 身份头"
    );
    assert!(
        req.headers
            .iter()
            .any(|(k, v)| k == "X-Auth-Kind" && v == "session"),
        "sync Http 须声明 session auth intent"
    );
}

/// 顺序不变性：先 Connected 后 hello，终态 Connected + connectionId 已存。
#[test]
fn order_connected_then_hello_terminal_connected() {
    let mut m = module_with_channel();
    let mut sink = EffectSink::new();
    m.handle(&Tick::Connected(TransportId::from_raw(0)), 1000, &mut sink)
        .unwrap();
    m.handle(&hello_frame("C1"), 1001, &mut sink).unwrap();
    assert_eq!(m.state.conn, ConnState::Connected);
    assert_eq!(m.state.connection_id.as_deref(), Some("C1"));
}
