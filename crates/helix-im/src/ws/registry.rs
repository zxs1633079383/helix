use std::collections::HashMap;
use std::sync::OnceLock;

use crate::error::ImError;
use helix_core::EffectSink;

use super::context::ImWsContext;
use super::frame::WsFrame;

pub(crate) trait WsMessageHandler: Sync {
    fn action(&self) -> &'static str;

    fn handle(
        &self,
        ctx: &mut ImWsContext<'_>,
        frame: &WsFrame,
        out: &mut EffectSink,
    ) -> Result<(), ImError>;
}

pub(crate) struct WsHandlerRegistration {
    pub(crate) action: &'static str,
    pub(crate) handler: &'static dyn WsMessageHandler,
}

inventory::collect!(WsHandlerRegistration);

// Presence is module-owned because its leases and HTTP replies share one ephemeral state.
const EXPECTED_WS_HANDLER_COUNT: usize = 31;

static WS_HANDLER_MAP: OnceLock<
    Result<HashMap<&'static str, &'static dyn WsMessageHandler>, ImError>,
> = OnceLock::new();

pub(crate) fn registry(
) -> Result<&'static HashMap<&'static str, &'static dyn WsMessageHandler>, ImError> {
    #[cfg(target_arch = "wasm32")]
    super::handlers::force_link_inventory();
    WS_HANDLER_MAP
        .get_or_init(|| {
            let registry = build_registry(inventory::iter::<WsHandlerRegistration>)?;
            if registry.len() != EXPECTED_WS_HANDLER_COUNT {
                return Err(ImError::IncompleteWsHandlerRegistry {
                    expected: EXPECTED_WS_HANDLER_COUNT,
                    actual: registry.len(),
                });
            }
            Ok(registry)
        })
        .as_ref()
        .map_err(Clone::clone)
}

fn build_registry(
    entries: impl IntoIterator<Item = &'static WsHandlerRegistration>,
) -> Result<HashMap<&'static str, &'static dyn WsMessageHandler>, ImError> {
    let entries = entries.into_iter();
    let mut registry = HashMap::with_capacity(entries.size_hint().0);

    for entry in entries {
        let handler_action = entry.handler.action();
        if entry.action != handler_action {
            return Err(ImError::InvalidWsHandlerRegistration {
                registered: entry.action.to_string(),
                handler: handler_action.to_string(),
            });
        }

        if registry.insert(entry.action, entry.handler).is_some() {
            return Err(ImError::DuplicateWsAction(entry.action.to_string()));
        }
    }

    Ok(registry)
}

/// 已注册 action 使用静态键作为诊断来源，避免每条正常 WS 推送分配字符串。
pub(crate) fn canonical_action(action: &str) -> Option<&'static str> {
    registry().ok()?.get_key_value(action).map(|(key, _)| *key)
}

pub(crate) fn dispatch_ws(
    ctx: &mut ImWsContext<'_>,
    frame: &WsFrame,
    out: &mut EffectSink,
) -> Result<(), ImError> {
    // —— HOP② 常驻日志（全链 echo 诊断·② helix 收到）——————————————————————————
    // 真源：native engine_loop 经 Tick::Inbound 直泵 helix-im，**绕过** Recording::recv() 的
    // ws-recv tee（helix-driver-native/transport.rs:8）→ run.jsonl 永无 ws-recv 帧（②面盲点）。
    // 在 dispatch 入口常驻 tracing 一行，让「后端是否把帧推到了 helix」可观测（落 run-app.log）。
    let action = frame.action()?;
    tracing::info!(
        hop = "2-ws-recv",
        action = %action,
        delivery_seq = frame.delivery_seq(),
        event_seq = frame.event_seq().map(|s| s.0),
        "HOP2 ws frame received by helix dispatch"
    );

    let handler = registry()?
        .get(action)
        .ok_or_else(|| ImError::UnsupportedWsAction(action.to_string()))?;

    handler.handle(ctx, frame, out)
}

#[cfg(test)]
mod tests {
    use helix_core::effect::StorageOp;
    use helix_core::{Effect, EffectSink};

    use crate::channel::Channel;
    use crate::pending_send::PendingSend;
    use crate::state::ImState;
    use crate::state::{ChannelId, ConnState, SendStatus, Seq, ServerId, TemporaryId};

    use super::*;

    struct TestHandler {
        action: &'static str,
    }

    impl WsMessageHandler for TestHandler {
        fn action(&self) -> &'static str {
            self.action
        }

        fn handle(
            &self,
            _ctx: &mut ImWsContext<'_>,
            _frame: &WsFrame,
            _out: &mut EffectSink,
        ) -> Result<(), ImError> {
            Ok(())
        }
    }

    static POSTED_HANDLER: TestHandler = TestHandler { action: "posted" };
    static DUPLICATE_POSTED_HANDLER: TestHandler = TestHandler { action: "posted" };
    static MISMATCHED_HANDLER: TestHandler = TestHandler {
        action: "post_update",
    };

    fn frame_from_json(value: serde_json::Value) -> WsFrame {
        let bytes = serde_json::to_vec(&value).unwrap();
        WsFrame::parse(&bytes).unwrap()
    }

    fn channel_id(n: u64) -> ChannelId {
        crate::state::test_channel_id(n)
    }

    fn channel_id_string(n: u64) -> String {
        channel_id(n).as_str().to_string()
    }

    fn server_id(n: u64) -> ServerId {
        crate::state::test_server_id(n)
    }

    fn server_id_string(n: u64) -> String {
        server_id(n).as_str().to_string()
    }

    fn emit_body(effect: &Effect) -> Option<String> {
        match effect {
            Effect::Emit { event } => Some(String::from_utf8_lossy(event.0.as_ref()).into_owned()),
            _ => None,
        }
    }

    fn sync_http_channel_ids(out: &EffectSink) -> Vec<String> {
        out.as_slice()
            .iter()
            .filter_map(|effect| match effect {
                Effect::Http { req, .. } if req.url.ends_with("/channel/sync/notify") => {
                    let body: serde_json::Value =
                        serde_json::from_slice(req.body.as_ref().unwrap()).unwrap();
                    Some(
                        body["cursors"][0]["channelId"]
                            .as_str()
                            .unwrap()
                            .to_string(),
                    )
                }
                _ => None,
            })
            .collect()
    }

    #[test]
    fn build_registry_rejects_duplicate_actions() {
        static ENTRIES: &[WsHandlerRegistration] = &[
            WsHandlerRegistration {
                action: "posted",
                handler: &POSTED_HANDLER,
            },
            WsHandlerRegistration {
                action: "posted",
                handler: &DUPLICATE_POSTED_HANDLER,
            },
        ];

        let err = match build_registry(ENTRIES) {
            Ok(_) => panic!("duplicate actions should fail"),
            Err(err) => err,
        };

        assert!(matches!(err, ImError::DuplicateWsAction(action) if action == "posted"));
    }

    #[test]
    fn build_registry_rejects_registration_action_mismatch() {
        static ENTRIES: &[WsHandlerRegistration] = &[WsHandlerRegistration {
            action: "posted",
            handler: &MISMATCHED_HANDLER,
        }];

        let err = match build_registry(ENTRIES) {
            Ok(_) => panic!("registration action mismatch should fail"),
            Err(err) => err,
        };

        assert!(matches!(
            err,
            ImError::InvalidWsHandlerRegistration { registered, handler }
                if registered == "posted" && handler == "post_update"
        ));
    }

    #[test]
    fn inventory_registry_has_unique_actions() {
        let expected = inventory::iter::<WsHandlerRegistration>.into_iter().count();
        let registry = build_registry(inventory::iter::<WsHandlerRegistration>)
            .expect("inventory WS registry must be valid");

        assert_eq!(registry.len(), expected);
        assert_eq!(
            expected, EXPECTED_WS_HANDLER_COUNT,
            "WS handler inventory 注册数不应静默漂移"
        );
    }

    #[test]
    fn member_projection_routes_to_revision_guarded_atomic_persist() {
        let mut state = ImState::new();
        let channel_id = channel_id(29);
        let mut alloc_corr = || helix_core::Correlation::from_raw(29);
        let mut ctx = ImWsContext::new(&mut state, 1_000, "", "viewer-29", &mut alloc_corr);
        let frame = frame_from_json(serde_json::json!({
            "seq": 999,
            "action": "member_projection",
            "data": {
                "channelId": channel_id.as_str(),
                "userId": "viewer-29",
                "projectionRevision": 3,
                "effectId": "effect-3",
                "unreadCount": 1,
                "lastReadSeq": 7
            }
        }));
        let mut out = EffectSink::new();

        dispatch_ws(&mut ctx, &frame, &mut out).unwrap();

        let [Effect::PersistAtomic { corr, ops }] = out.as_slice() else {
            panic!("canonical projection must use one atomic persist");
        };
        assert_eq!(corr.raw(), 29);
        assert!(matches!(
            ops.as_slice(),
            [StorageOp::BatchUpsert(_), StorageOp::ScopedGuardedBump(_)]
        ));
        assert!(matches!(
            ctx.state.corr_map.get(corr),
            Some(crate::state::CorrelationContext::MemberProjectionPersist {
                expected_revision: 3,
                expected_effect_id,
                ..
            }) if expected_effect_id == "effect-3"
        ));
    }

    /// 零 revision 或空 effect identity 必须显式失败，禁止在线成员投影静默丢弃首个事件。
    #[test]
    fn member_projection_rejects_uninitialized_identity_without_effects() {
        let mut state = ImState::new();
        let channel_id = channel_id(291);
        let mut alloc_corr = || helix_core::Correlation::from_raw(291);
        let mut ctx = ImWsContext::new(&mut state, 1_000, "", "viewer-291", &mut alloc_corr);
        let frame = frame_from_json(serde_json::json!({
            "seq": 1_000,
            "action": "member_projection",
            "data": {
                "channelId": channel_id.as_str(),
                "userId": "viewer-291",
                "projectionRevision": 0,
                "effectId": "",
                "unreadCount": 1
            }
        }));
        let mut out = EffectSink::new();

        let err = dispatch_ws(&mut ctx, &frame, &mut out).unwrap_err();

        assert!(matches!(
            err,
            ImError::Parse(message)
                if message == "member projection missing revision"
        ));
        assert!(out.as_slice().is_empty());
    }

    #[test]
    fn canonical_stream_waits_for_atomic_ack_before_cursor_or_emit() {
        let mut state = ImState::new();
        let channel_id = channel_id(30);
        state
            .channels
            .insert(channel_id, Channel::new(channel_id, 0));
        let mut alloc_corr = || helix_core::Correlation::from_raw(30);
        let mut ctx = ImWsContext::new(&mut state, 1_000, "", "viewer-30", &mut alloc_corr);
        let frame = frame_from_json(serde_json::json!({
            "seq": 999,
            "action": "channel_stream_event",
            "data": {
                "channelId": channel_id.as_str(),
                "streamSeq": 1,
                "eventId": "event-1",
                "effectId": "effect-1",
                "eventType": 1,
                "actorId": "author-1",
                "msgId": server_id_string(1),
                "redacted": false,
                "payload": {
                    "id": server_id_string(1),
                    "channelId": channel_id.as_str(),
                    "userId": "author-1",
                    "message": "hello",
                    "createAt": 1
                }
            }
        }));
        let mut out = EffectSink::new();

        dispatch_ws(&mut ctx, &frame, &mut out).unwrap();

        assert_eq!(ctx.state.channels[&channel_id].cursor.value(), Seq(0));
        assert!(matches!(out.as_slice(), [Effect::PersistAtomic { .. }]));
    }

    #[test]
    fn canonical_and_legacy_terminal_share_one_inflight_write() {
        let mut state = ImState::new();
        let channel_id = channel_id(32);
        state
            .channels
            .insert(channel_id, Channel::new(channel_id, 0));
        let mut next_corr = 40;
        let mut alloc_corr = || {
            let corr = helix_core::Correlation::from_raw(next_corr);
            next_corr += 1;
            corr
        };
        let mut ctx = ImWsContext::new(&mut state, 1_000, "", "viewer-32", &mut alloc_corr);
        let mut out = EffectSink::new();
        dispatch_ws(
            &mut ctx,
            &frame_from_json(serde_json::json!({
                "action": "channel_stream_event",
                "data": {
                    "channelId": channel_id.as_str(),
                    "streamSeq": 1,
                    "eventId": "terminal-1",
                    "eventType": 7,
                    "redacted": false,
                    "payload": {"state": "closed"}
                }
            })),
            &mut out,
        )
        .unwrap();
        dispatch_ws(
            &mut ctx,
            &frame_from_json(serde_json::json!({
                "action": "channel_close",
                "data": {
                    "id": "terminal-1",
                    "channelId": channel_id.as_str(),
                    "event_seq": 1,
                    "event_type": 7,
                    "payload": {"state": "closed"},
                    "deleteAt": 1
                }
            })),
            &mut out,
        )
        .unwrap();

        assert_eq!(
            out.as_slice()
                .iter()
                .filter(|effect| matches!(effect, Effect::PersistAtomic { .. }))
                .count(),
            1
        );
        assert_eq!(ctx.state.channels[&channel_id].cursor.value(), Seq(0));
    }

    #[test]
    fn dispatch_ws_returns_unsupported_ws_action_for_unknown_action() {
        let mut state = ImState::new();
        let mut alloc_corr = || helix_core::Correlation::from_raw(1);
        let mut ctx = ImWsContext::new(&mut state, 0, "", "", &mut alloc_corr);
        let frame = WsFrame::parse(br#"{"action":"missing"}"#).unwrap();
        let mut out = EffectSink::new();

        let err = dispatch_ws(&mut ctx, &frame, &mut out).unwrap_err();

        assert!(matches!(err, ImError::UnsupportedWsAction(action) if action == "missing"));
    }

    #[test]
    fn dispatch_ws_ignored_actions_are_noop() {
        let mut state = ImState::new();
        let channel_id = channel_id(31);
        state
            .channels
            .insert(channel_id, Channel::new(channel_id, 77));
        state.conn = ConnState::Connecting;

        let mut alloc_corr = || helix_core::Correlation::from_raw(1);
        let mut ctx = ImWsContext::new(&mut state, 1_000, "", "", &mut alloc_corr);
        let ignored_actions = ["status_change", "multiple_channels_viewed", "typing"];

        for action in ignored_actions {
            let frame = frame_from_json(serde_json::json!({
                "v": 1,
                "action": action,
                "data": { "channel_id": channel_id.as_str() }
            }));
            let mut out = EffectSink::new();

            dispatch_ws(&mut ctx, &frame, &mut out).unwrap();

            assert!(
                out.as_slice().is_empty(),
                "{action} should not produce effects, got: {:?}",
                out.as_slice()
            );
            assert_eq!(
                ctx.state.channels.get(&channel_id).unwrap().cursor.value(),
                Seq(77),
                "{action} should not advance channel cursor"
            );
            assert_eq!(
                ctx.state.conn,
                ConnState::Connecting,
                "{action} should not change connection state"
            );
            assert!(
                ctx.state.pending_sends.is_empty(),
                "{action} should not touch pending sends"
            );
        }
    }

    // ─── P4 dead-action（posted / post_urgent / post_edited）graceful no-op ─────
    // P4 下线 `posted` reconcile（echo 真路径只走 `post`，见 handlers/posted.rs 头注）。
    // 这三个遗留 action 现注册为 no-op：经 registry 路由不冒 UnsupportedWsAction，零 Effect。
    #[test]
    fn dispatch_ws_dead_actions_route_as_noop() {
        let mut state = ImState::new();
        let channel_id = channel_id(21);
        state
            .channels
            .insert(channel_id, Channel::new(channel_id, 3));
        // 在途 send：dead action posted 帧即便带 temporary_id 也**不得**对账（reconcile 已下线）。
        let tmp_reg = TemporaryId("tmp-registry".to_string());
        let ps = PendingSend::new(tmp_reg.clone(), helix_core::TimerId::from_raw(44), None);
        state.pending_sends.insert(tmp_reg, ps);

        let mut alloc_corr = || helix_core::Correlation::from_raw(1);
        let mut ctx = ImWsContext::new(&mut state, 1_000, "", "", &mut alloc_corr);

        for action in ["posted", "post_urgent", "post_edited"] {
            // 即便构造「最像 echo」的帧（带 temporary_id + 合法 server_id + event_seq），也 no-op。
            let frame = frame_from_json(serde_json::json!({
                "v": 1,
                "action": action,
                "data": {
                    "id": server_id_string(42),
                    "temporary_id": "tmp-registry",
                    "channel_id": channel_id.as_str(),
                    "event_seq": 9,
                    "message": "hello"
                }
            }));
            let mut out = EffectSink::new();

            // 路由成功（不冒 UnsupportedWsAction），零 Effect。
            dispatch_ws(&mut ctx, &frame, &mut out).unwrap();
            assert!(
                out.as_slice().is_empty(),
                "{action} is a dead action → no-op, got: {:?}",
                out.as_slice()
            );
        }

        // pending send 不被对账（仍未推进，reconcile 真路径 = post.rs）。
        assert_eq!(
            ctx.state
                .pending_sends
                .get(&TemporaryId("tmp-registry".to_string()))
                .map(|ps| ps.status),
            Some(SendStatus::Local),
            "dead action posted must not reconcile pending send (echo 只走 post)"
        );
        // cursor 不被 posted 帧推进。
        assert_eq!(
            ctx.state.channels.get(&channel_id).unwrap().cursor.value(),
            Seq(3),
            "dead action posted must not advance cursor"
        );
    }

    #[test]
    fn dispatch_ws_routes_hello_through_registry() {
        let mut state = ImState::new();
        let channel_id = crate::state::test_channel_id(7);
        state
            .channels
            .insert(channel_id, Channel::new(channel_id, 42));
        state.increment_fetched.insert(channel_id);
        // 该单元测试直接构造已载入的频道；等价于 on_start 的 channel 投影扫描已完成。
        state.startup_channel_projection_ready = true;

        let mut next_corr = 91;
        let mut alloc_corr = || {
            let corr = helix_core::Correlation::from_raw(next_corr);
            next_corr += 1;
            corr
        };
        let mut ctx = ImWsContext::new(
            &mut state,
            1_000,
            "https://example.test/api",
            "viewer",
            &mut alloc_corr,
        );
        let frame =
            WsFrame::parse(br#"{"v":1,"action":"hello","data":{"connectionId":"CONN_REG"}}"#)
                .unwrap();
        let mut out = EffectSink::new();

        dispatch_ws(&mut ctx, &frame, &mut out).unwrap();

        assert_eq!(ctx.state.conn, ConnState::Connected);
        assert_eq!(ctx.state.connection_id.as_deref(), Some("CONN_REG"));
        assert!(
            ctx.state.recovery_session.is_collecting_for("viewer"),
            "production registry hello must establish the actor recovery epoch"
        );
        assert!(
            ctx.state.increment_fetched.is_empty(),
            "hello should reset the previous increment batch"
        );

        let mut saw_established = false;
        let mut saw_message_timestamp_scan = false;
        for effect in out.as_slice() {
            match effect {
                Effect::Emit { event } => {
                    let body = String::from_utf8_lossy(event.0.as_ref());
                    saw_established |=
                        body.contains("im:connection:established") && body.contains("CONN_REG");
                    assert!(!body.contains("im:channels:loaded"));
                }
                Effect::Persist { corr, ops } => {
                    saw_message_timestamp_scan = true;
                    assert_eq!(corr.raw(), 91);
                    let [StorageOp::ScopedMax(spec)] = ops.as_slice() else {
                        panic!("hello watermark must be one scoped max operation");
                    };
                    assert_eq!(spec.table, "message");
                    assert_eq!(spec.scope_col, "channel_id");
                    assert!(spec.scope_values.is_empty());
                    assert_eq!(spec.value_col, "create_at");
                    assert_eq!(spec.result_alias, "create_at");
                }
                Effect::Http { .. } => {
                    panic!("hello must wait for the message watermark scan before HTTP");
                }
                _ => {}
            }
        }
        assert!(saw_established, "missing connection established emit");
        assert!(
            saw_message_timestamp_scan,
            "missing message watermark scan before increment HTTP"
        );
    }

    #[test]
    fn dispatch_ws_increment_channel_registers_channel_and_emits_increment() {
        let mut state = ImState::new();
        let mut alloc_corr = || helix_core::Correlation::from_raw(1);
        let channel_id = channel_id(77);
        let mut ctx = ImWsContext::new(&mut state, 1_000, "", "", &mut alloc_corr);
        let frame = frame_from_json(serde_json::json!({
            "action": "increment_channel",
            "data": {
                "id": channel_id.as_str(),
                "lastEventSeq": 42,
                "needSync": true,
                "mentionList": [],
                "urgentPostList": []
            }
        }));
        let mut out = EffectSink::new();

        dispatch_ws(&mut ctx, &frame, &mut out).unwrap();

        let channel = ctx
            .state
            .channels
            .get(&channel_id)
            .expect("increment should register a missing channel");
        // 修 todo#1：冷启动真新 channel 内存 cursor 种子 = 0（本地确认水位），**不是** lastEventSeq(42)。
        // lastEventSeq 只喂 increment_target（gap / B2-heal stuck 检测），与 cursor 严格解耦。
        assert_eq!(channel.cursor.value(), Seq(0));
        assert!(ctx.state.increment_fetched.contains(&channel_id));
        assert!(!ctx.state.need_sync_skip.contains(&channel_id));
        assert_eq!(ctx.state.increment_target.get(&channel_id), Some(&Seq(42)));
        assert!(out.as_slice().is_empty(), "increment 帧只应进入待提交批次");
        assert!(!ctx.state.pending_increment_ops.is_empty());
        assert_eq!(ctx.state.pending_increment_projections.len(), 1);
    }

    #[test]
    fn dispatch_ws_global_increment_end_syncs_fetched_minus_skip() {
        let mut state = ImState::new();
        let sync_channel = channel_id(10);
        let skipped_channel = channel_id(12);
        state
            .channels
            .insert(sync_channel, Channel::new(sync_channel, 5));
        state
            .channels
            .insert(skipped_channel, Channel::new(skipped_channel, 3));
        state.increment_fetched.insert(sync_channel);
        state.increment_fetched.insert(skipped_channel);
        state.need_sync_skip.insert(skipped_channel);
        state.connection_id = Some("CONN_INC".to_string());

        let mut next_corr = 7;
        let mut alloc_corr = || {
            let corr = helix_core::Correlation::from_raw(next_corr);
            next_corr += 1;
            corr
        };
        let mut ctx = ImWsContext::new(
            &mut state,
            1_000,
            "https://example.test/api",
            "",
            &mut alloc_corr,
        );
        let frame = frame_from_json(serde_json::json!({
            "action": "increment_channel_end",
            "data": {}
        }));
        let mut out = EffectSink::new();

        dispatch_ws(&mut ctx, &frame, &mut out).unwrap();

        assert_eq!(sync_http_channel_ids(&out), vec![sync_channel.as_str()]);
        assert!(out.as_slice().iter().any(|effect| {
            matches!(effect, Effect::PersistAtomic { corr, .. } if corr.raw() == 8)
        }));
        assert_eq!(
            ctx.state
                .corr_map
                .get(&helix_core::Correlation::from_raw(7)),
            Some(&crate::state::CorrelationContext::SyncPull {
                channel_id: sync_channel,
                trigger: crate::state::SyncTrigger::Routine,
            })
        );
        assert_eq!(
            ctx.state
                .corr_map
                .get(&helix_core::Correlation::from_raw(8)),
            Some(&crate::state::CorrelationContext::IncrementBatchPersist {
                projections: Vec::new(),
                batch_id: None,
            })
        );
    }

    // 同一 global end 只封口一次；下一批新 increment 到达后必须重新允许 ready。
    #[test]
    fn channel_sync_global_increment_end_reopens_after_new_batch() {
        let mut state = ImState::new();
        let mut next_corr = 1;
        let mut alloc_corr = || {
            let corr = helix_core::Correlation::from_raw(next_corr);
            next_corr += 1;
            corr
        };
        let mut ctx = ImWsContext::new(
            &mut state,
            1_000,
            "https://example.test/api",
            "user-a",
            &mut alloc_corr,
        );
        let channel_a = channel_id(21);
        let channel_b = channel_id(22);
        let increment_a = frame_from_json(serde_json::json!({
            "action": "increment_channel",
            "data": {
                "id": channel_a.as_str(),
                "lastEventSeq": 1,
                "needSync": false,
                "mentionList": [],
                "urgentPostList": []
            }
        }));
        let end = frame_from_json(serde_json::json!({
            "action": "increment_channel_end",
            "data": {}
        }));

        let mut out = EffectSink::new();
        dispatch_ws(&mut ctx, &increment_a, &mut out).unwrap();
        out.clear();
        dispatch_ws(&mut ctx, &end, &mut out).unwrap();
        assert!(out
            .as_slice()
            .iter()
            .any(|effect| matches!(effect, Effect::PersistAtomic { .. })));
        assert!(!ctx.state.channel_sync_batch_pending);

        out.clear();
        dispatch_ws(&mut ctx, &end, &mut out).unwrap();
        assert!(
            out.as_slice().is_empty(),
            "duplicate global end must be ignored"
        );

        let increment_b = frame_from_json(serde_json::json!({
            "action": "increment_channel",
            "data": {
                "id": channel_b.as_str(),
                "lastEventSeq": 2,
                "needSync": false,
                "mentionList": [],
                "urgentPostList": []
            }
        }));
        dispatch_ws(&mut ctx, &increment_b, &mut out).unwrap();
        out.clear();
        dispatch_ws(&mut ctx, &end, &mut out).unwrap();
        assert!(out
            .as_slice()
            .iter()
            .any(|effect| matches!(effect, Effect::PersistAtomic { .. })));
        assert!(!ctx.state.channel_sync_batch_pending);
    }

    /// 单群资料持久化不会开启全局输入批次；真正的新WS increment仍必须重新开批。
    #[test]
    fn single_channel_hydration_preserves_global_batch_boundary() {
        let mut state = ImState::new();
        state.channel_sync_batch_pending = false;
        let channel = channel_id(23);
        let raw = serde_json::json!({
            "id": channel.as_str(), "lastEventSeq": 0, "needSync": false,
            "mentionList": [], "urgentPostList": []
        });
        let inc = crate::ws::parser::parse_increment_channel(&raw).unwrap();
        let mut alloc = || helix_core::Correlation::from_raw(10);
        let mut ctx = ImWsContext::new(
            &mut state,
            1_000,
            "https://example.test/api",
            "user-a",
            &mut alloc,
        );
        let mut out = EffectSink::new();
        assert!(
            super::super::handlers::increment_channel::apply_increment_hydration(
                &mut ctx,
                &inc,
                helix_core::Correlation::from_raw(11),
                &mut out
            )
        );
        assert!(
            !ctx.state.channel_sync_batch_pending,
            "single-channel hydration must not reopen a sealed global inventory batch"
        );
        assert!(out
            .as_slice()
            .iter()
            .any(|effect| matches!(effect, Effect::PersistAtomic { .. })));

        out.clear();
        super::super::handlers::increment_channel::apply_increment(&mut ctx, &inc, &mut out);
        assert!(
            ctx.state.channel_sync_batch_pending,
            "new WS input must reopen the next batch"
        );
        super::super::handlers::increment_channel::commit_increment_state(&mut ctx, &inc);
        assert!(
            ctx.state.channel_sync_batch_pending,
            "state application must preserve an open batch"
        );
    }

    #[test]
    fn dispatch_ws_invalid_increment_end_channel_id_preserves_global_end_compatibility() {
        let mut state = ImState::new();
        let sync_channel = channel_id(10);
        state
            .channels
            .insert(sync_channel, Channel::new(sync_channel, 5));
        state.increment_fetched.insert(sync_channel);

        let mut next_corr = 7;
        let mut alloc_corr = || {
            let corr = helix_core::Correlation::from_raw(next_corr);
            next_corr += 1;
            corr
        };
        let mut ctx = ImWsContext::new(
            &mut state,
            1_000,
            "https://example.test/api",
            "",
            &mut alloc_corr,
        );
        let frame = frame_from_json(serde_json::json!({
            "action": "increment_channel_end",
            "data": { "channelId": "not-a-valid-channel-id" }
        }));
        let mut out = EffectSink::new();

        dispatch_ws(&mut ctx, &frame, &mut out).unwrap();

        assert_eq!(sync_http_channel_ids(&out), vec![sync_channel.as_str()]);
        assert!(out
            .as_slice()
            .iter()
            .any(|effect| matches!(effect, Effect::PersistAtomic { .. })));
    }

    #[test]
    fn dispatch_ws_subtopic_increment_end_emits_update_without_sync() {
        let mut state = ImState::new();
        let mut alloc_corr = || helix_core::Correlation::from_raw(1);
        let mut ctx = ImWsContext::new(
            &mut state,
            1_000,
            "https://example.test/api",
            "",
            &mut alloc_corr,
        );
        let frame = frame_from_json(serde_json::json!({
            "action": "increment_channel_end",
            "data": { "channelId": channel_id_string(99) }
        }));
        let mut out = EffectSink::new();

        dispatch_ws(&mut ctx, &frame, &mut out).unwrap();

        assert!(sync_http_channel_ids(&out).is_empty());
        assert!(
            out.as_slice().iter().filter_map(emit_body).any(|body| {
                body.contains("im:channel:update") && body.contains(&channel_id_string(99))
            }),
            "subtopic end should emit channel update, got: {:?}",
            out.as_slice()
        );
    }

    // 验证话题收尾只接受同一批次，并在成功时进入统一 Persist 屏障。
    #[test]
    fn dispatch_ws_scoped_subtopic_end_requires_matching_batch_and_persists_marker() {
        let mut state = ImState::new();
        let mut next_corr = 1;
        let mut alloc_corr = || {
            let corr = helix_core::Correlation::from_raw(next_corr);
            next_corr += 1;
            corr
        };
        let channel_id = channel_id(101);
        let mut ctx = ImWsContext::new(&mut state, 1_000, "", "", &mut alloc_corr);

        let increment = frame_from_json(serde_json::json!({
            "action": "increment_channel",
            "data": {
                "id": channel_id.as_str(),
                "batchId": "batch-a",
                "lastEventSeq": 3,
                "mentionList": [],
                "urgentPostList": []
            }
        }));
        let mut increment_out = EffectSink::new();
        dispatch_ws(&mut ctx, &increment, &mut increment_out).unwrap();
        assert!(increment_out.as_slice().is_empty());
        assert_eq!(
            ctx.state.pending_increment_batch_id.as_deref(),
            Some("batch-a")
        );

        let stale_end = frame_from_json(serde_json::json!({
            "action": "increment_channel_end",
            "data": {
                "scope": "subtopics",
                "channelId": channel_id.as_str(),
                "batchId": "batch-old"
            }
        }));
        let mut stale_out = EffectSink::new();
        dispatch_ws(&mut ctx, &stale_end, &mut stale_out).unwrap();
        assert!(stale_out.as_slice().is_empty());
        assert_eq!(
            ctx.state.pending_increment_batch_id.as_deref(),
            Some("batch-a")
        );

        let matching_end = frame_from_json(serde_json::json!({
            "action": "increment_channel_end",
            "data": {
                "scope": "subtopics",
                "channelId": channel_id.as_str(),
                "batchId": "batch-a"
            }
        }));
        let mut matching_out = EffectSink::new();
        dispatch_ws(&mut ctx, &matching_end, &mut matching_out).unwrap();
        assert!(matching_out
            .as_slice()
            .iter()
            .any(|effect| matches!(effect, Effect::Persist { .. })));
        assert!(matching_out
            .as_slice()
            .iter()
            .all(|effect| !matches!(effect, Effect::Http { .. })));
        assert!(ctx.state.subtopic_sync_active.is_some());
    }

    // 验证声明为普通频道的收尾帧携带父频道时 fail-closed，不误触发话题或列表同步。
    #[test]
    fn dispatch_ws_declared_channels_scope_rejects_parent_channel_id() {
        let mut state = ImState::new();
        let mut alloc_corr = || helix_core::Correlation::from_raw(1);
        let mut ctx = ImWsContext::new(&mut state, 1_000, "", "", &mut alloc_corr);
        let frame = frame_from_json(serde_json::json!({
            "action": "increment_channel_end",
            "data": {
                "scope": "channels",
                "channelId": channel_id_string(102),
                "batchId": "batch-a"
            }
        }));
        let mut out = EffectSink::new();
        dispatch_ws(&mut ctx, &frame, &mut out).unwrap();
        assert!(out.as_slice().is_empty());
    }

    // 验证普通频道收尾也必须匹配 increment 批次，旧 end 不得提前推进持久化屏障。
    /// 旧收尾不封口，新收尾将原批次标识带到真实持久化回执。
    #[test]
    fn dispatch_ws_declared_channels_scope_rejects_stale_batch_id() {
        let mut state = ImState::new();
        let mut next_corr = 1;
        let mut alloc_corr = || {
            let corr = helix_core::Correlation::from_raw(next_corr);
            next_corr += 1;
            corr
        };
        let channel_id = channel_id(103);
        let mut ctx = ImWsContext::new(&mut state, 1_000, "", "", &mut alloc_corr);

        let increment = frame_from_json(serde_json::json!({
            "action": "increment_channel",
            "data": {
                "id": channel_id.as_str(),
                "batchId": "batch-current",
                "lastEventSeq": 4,
                "mentionList": [],
                "urgentPostList": []
            }
        }));
        let mut increment_out = EffectSink::new();
        dispatch_ws(&mut ctx, &increment, &mut increment_out).unwrap();

        let stale_end = frame_from_json(serde_json::json!({
            "action": "increment_channel_end",
            "data": {
                "scope": "channels",
                "batchId": "batch-stale"
            }
        }));
        let mut stale_out = EffectSink::new();
        dispatch_ws(&mut ctx, &stale_end, &mut stale_out).unwrap();
        assert!(stale_out.as_slice().is_empty());
        assert_eq!(
            ctx.state.pending_increment_batch_id.as_deref(),
            Some("batch-current")
        );
        let single_end = frame_from_json(serde_json::json!({
            "action": "increment_channel_end", "data": { "channelId": channel_id.as_str() }
        }));
        dispatch_ws(&mut ctx, &single_end, &mut stale_out).unwrap();
        assert_eq!(
            ctx.state.pending_increment_batch_id.as_deref(),
            Some("batch-current")
        );
        let valid_end = frame_from_json(serde_json::json!({
            "action": "increment_channel_end",
            "data": { "scope": "channels", "batchId": "batch-current" }
        }));
        dispatch_ws(&mut ctx, &valid_end, &mut stale_out).unwrap();
        assert!(ctx.state.pending_increment_batch_id.is_none());
        assert!(ctx.state.corr_map.values().any(|context| matches!(context,
            crate::state::CorrelationContext::IncrementBatchPersist { batch_id: Some(id), .. } if id == "batch-current"
        )));
    }

    #[test]
    fn dispatch_ws_invalid_increment_channel_is_silent_noop() {
        let mut state = ImState::new();
        let mut alloc_corr = || helix_core::Correlation::from_raw(1);
        let mut ctx = ImWsContext::new(&mut state, 1_000, "", "", &mut alloc_corr);
        let frame = frame_from_json(serde_json::json!({
            "action": "increment_channel",
            "data": { "id": "tooshort", "lastEventSeq": 1 }
        }));
        let mut out = EffectSink::new();

        dispatch_ws(&mut ctx, &frame, &mut out).unwrap();

        assert!(ctx.state.channels.is_empty());
        assert!(ctx.state.increment_fetched.is_empty());
        assert!(out.as_slice().is_empty());
    }
}
