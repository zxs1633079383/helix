use std::collections::HashMap;
use std::sync::OnceLock;

use linkme::distributed_slice;

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

#[distributed_slice]
pub(crate) static WS_HANDLERS: [WsHandlerRegistration];

static WS_HANDLER_MAP: OnceLock<
    Result<HashMap<&'static str, &'static dyn WsMessageHandler>, ImError>,
> = OnceLock::new();

pub(crate) fn registry(
) -> Result<&'static HashMap<&'static str, &'static dyn WsMessageHandler>, ImError> {
    WS_HANDLER_MAP
        .get_or_init(|| build_registry(&WS_HANDLERS))
        .as_ref()
        .map_err(Clone::clone)
}

fn build_registry(
    entries: &'static [WsHandlerRegistration],
) -> Result<HashMap<&'static str, &'static dyn WsMessageHandler>, ImError> {
    let mut registry = HashMap::with_capacity(entries.len());

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

pub(crate) fn dispatch_ws(
    ctx: &mut ImWsContext<'_>,
    frame: &WsFrame,
    out: &mut EffectSink,
) -> Result<(), ImError> {
    let handler = registry()?
        .get(frame.action())
        .ok_or_else(|| ImError::UnsupportedWsAction(frame.action().to_string()))?;

    handler.handle(ctx, frame, out)
}

#[cfg(test)]
mod tests {
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
        ChannelId::from_test_u64(n)
    }

    fn channel_id_string(n: u64) -> String {
        channel_id(n).as_str().to_string()
    }

    fn server_id(n: u64) -> ServerId {
        ServerId::from_test_u64(n)
    }

    fn server_id_string(n: u64) -> String {
        server_id(n).as_str().to_string()
    }

    fn posted_echo_frame(channel_id: ChannelId, seq: u64, temporary_id: &str) -> WsFrame {
        frame_from_json(serde_json::json!({
            "v": 1,
            "action": "posted",
            "data": {
                "id": server_id_string(42),
                "temporary_id": temporary_id,
                "channel_id": channel_id.as_str(),
                "event_seq": seq,
                "message": "hello"
            }
        }))
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
    fn distributed_registry_has_unique_actions() {
        let registry = build_registry(&WS_HANDLERS).unwrap();

        assert_eq!(registry.len(), WS_HANDLERS.len());
    }

    #[test]
    fn dispatch_ws_returns_unsupported_ws_action_for_unknown_action() {
        let mut state = ImState::new();
        let mut alloc_corr = || helix_core::Correlation::from_raw(1);
        let mut ctx = ImWsContext::new(&mut state, 0, "", &mut alloc_corr);
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
        let mut ctx = ImWsContext::new(&mut state, 1_000, "", &mut alloc_corr);
        let ignored_actions = [
            "status_change",
            "multiple_channels_viewed",
            "typing",
            "presence",
        ];

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

    #[test]
    fn dispatch_ws_posted_echo_reconciles_pending_send_and_advances_cursor() {
        let mut state = ImState::new();
        let channel_id = channel_id(21);
        state
            .channels
            .insert(channel_id, Channel::new(channel_id, 3));
        state.pending_sends.insert(
            TemporaryId("tmp-registry".to_string()),
            PendingSend::new(
                TemporaryId("tmp-registry".to_string()),
                helix_core::TimerId::from_raw(44),
            ),
        );

        let mut next_corr = 501;
        let mut alloc_corr = || {
            let corr = helix_core::Correlation::from_raw(next_corr);
            next_corr += 1;
            corr
        };
        let mut ctx = ImWsContext::new(&mut state, 1_000, "", &mut alloc_corr);
        let frame = posted_echo_frame(channel_id, 9, "tmp-registry");
        let mut out = EffectSink::new();

        dispatch_ws(&mut ctx, &frame, &mut out).unwrap();

        let pending = ctx
            .state
            .pending_sends
            .get(&TemporaryId("tmp-registry".to_string()))
            .expect("pending send should remain tracked");
        assert_eq!(pending.status, SendStatus::Sent);
        assert_eq!(
            ctx.state.channels.get(&channel_id).unwrap().cursor.value(),
            Seq(9)
        );
        assert!(
            out.as_slice()
                .iter()
                .any(|effect| matches!(effect, Effect::Persist { corr, .. } if corr.raw() == 501)),
            "echo should reconcile through PendingSend with allocated corr, got: {:?}",
            out.as_slice()
        );
        assert!(
            out.as_slice()
                .iter()
                .any(|effect| matches!(effect, Effect::CancelTimer { id } if id.raw() == 44)),
            "echo should cancel pending send timeout, got: {:?}",
            out.as_slice()
        );
        assert!(
            out.as_slice()
                .iter()
                .any(|effect| matches!(effect, Effect::PersistFire { .. })),
            "echo should push advance_cursor when channel exists, got: {:?}",
            out.as_slice()
        );
    }

    #[test]
    fn dispatch_ws_posted_echo_advances_cursor_without_pending_send() {
        let mut state = ImState::new();
        let channel_id = channel_id(22);
        state
            .channels
            .insert(channel_id, Channel::new(channel_id, 3));

        let mut alloc_corr = || helix_core::Correlation::from_raw(1);
        let mut ctx = ImWsContext::new(&mut state, 1_000, "", &mut alloc_corr);
        let frame = frame_from_json(serde_json::json!({
            "v": 1,
            "action": "posted",
            "channel_event_seq": 99,
            "data": {
                "event_seq": 7,
                "post": {
                    "id": server_id_string(43),
                    "temporaryId": "tmp-no-pending",
                    "channelId": channel_id.as_str(),
                    "props": { "channel_event_seq": 8 }
                }
            }
        }));
        let mut out = EffectSink::new();

        dispatch_ws(&mut ctx, &frame, &mut out).unwrap();

        assert_eq!(
            ctx.state.channels.get(&channel_id).unwrap().cursor.value(),
            Seq(7),
            "data.event_seq must win over post.props and root channel_event_seq"
        );
        assert!(
            out.as_slice()
                .iter()
                .any(|effect| matches!(effect, Effect::PersistFire { .. })),
            "valid echo should advance cursor even without pending send, got: {:?}",
            out.as_slice()
        );
    }

    #[test]
    fn dispatch_ws_posted_echo_missing_channel_is_noop_without_error() {
        let mut state = ImState::new();
        let mut alloc_corr = || helix_core::Correlation::from_raw(1);
        let mut ctx = ImWsContext::new(&mut state, 1_000, "", &mut alloc_corr);
        let frame = frame_from_json(serde_json::json!({
            "v": 1,
            "action": "posted",
            "channel_event_seq": 11,
            "data": {
                "post": {
                    "id": server_id_string(44),
                    "temporary_id": "tmp-missing-channel-state",
                    "channel_id": channel_id_string(23)
                }
            }
        }));
        let mut out = EffectSink::new();

        dispatch_ws(&mut ctx, &frame, &mut out).unwrap();

        assert!(
            out.as_slice().is_empty(),
            "missing channel and pending send should produce no effects, got: {:?}",
            out.as_slice()
        );
    }

    #[test]
    fn dispatch_ws_posted_non_echo_gracefully_ignored() {
        let mut state = ImState::new();
        let mut alloc_corr = || helix_core::Correlation::from_raw(1);
        let mut ctx = ImWsContext::new(&mut state, 1_000, "", &mut alloc_corr);
        let frame = frame_from_json(serde_json::json!({
            "v": 1,
            "action": "posted",
            "data": {
                "id": server_id_string(42),
                "channel_id": channel_id_string(1),
                "event_seq": 4
            }
        }));
        let mut out = EffectSink::new();

        // 非 echo posted（无 temporary_id）→ 优雅忽略：Ok + 零 effect（对齐 dev Ignored 口径）
        dispatch_ws(&mut ctx, &frame, &mut out).unwrap();

        assert!(out.as_slice().is_empty());
    }

    #[test]
    fn dispatch_ws_posted_missing_echo_required_fields_is_invalid_frame() {
        let mut state = ImState::new();
        let channel_id = channel_id(21);
        state
            .channels
            .insert(channel_id, Channel::new(channel_id, 3));
        let mut alloc_corr = || helix_core::Correlation::from_raw(1);
        let mut ctx = ImWsContext::new(&mut state, 1_000, "", &mut alloc_corr);
        let frame = frame_from_json(serde_json::json!({
            "v": 1,
            "action": "posted",
            "data": {
                "id": server_id_string(42),
                "temporary_id": "tmp-missing-seq",
                "channel_id": channel_id.as_str(),
                "eventSeq": 99
            }
        }));
        let mut out = EffectSink::new();

        let err = dispatch_ws(&mut ctx, &frame, &mut out).unwrap_err();

        assert!(matches!(err, ImError::InvalidWsFrame(msg) if msg.contains("posted echo")));
        assert_eq!(
            ctx.state.channels.get(&channel_id).unwrap().cursor.value(),
            Seq(3),
            "eventSeq compatibility field must not classify posted echo"
        );
        assert!(out.as_slice().is_empty());
    }

    #[test]
    fn dispatch_ws_posted_invalid_echo_required_fields_is_invalid_frame() {
        let mut state = ImState::new();
        let mut alloc_corr = || helix_core::Correlation::from_raw(1);
        let mut ctx = ImWsContext::new(&mut state, 1_000, "", &mut alloc_corr);
        let frame = frame_from_json(serde_json::json!({
            "v": 1,
            "action": "posted",
            "data": {
                "id": "not-a-server-id",
                "temporary_id": "tmp-invalid",
                "channel_id": channel_id_string(1),
                "event_seq": 4
            }
        }));
        let mut out = EffectSink::new();

        let err = dispatch_ws(&mut ctx, &frame, &mut out).unwrap_err();

        assert!(matches!(err, ImError::InvalidWsFrame(msg) if msg.contains("posted echo")));
        assert!(out.as_slice().is_empty());
    }

    #[test]
    fn dispatch_ws_routes_legacy_no_action_event_through_registry() {
        let mut state = ImState::new();
        let channel_id = ChannelId::from_test_u64(1);
        state
            .channels
            .insert(channel_id, Channel::new(channel_id, 0));
        let mut alloc_corr = || helix_core::Correlation::from_raw(1);
        let mut ctx = ImWsContext::new(&mut state, 1_000, "", &mut alloc_corr);
        let raw = format!(
            r#"{{"event":"posted","data":{{"channel_id":"{}","seq":1,"event_type":1,"msg_id":"msg_1","actor_id":"user_1","created_at":1000000,"payload":null}}}}"#,
            channel_id.as_str()
        );
        let frame = WsFrame::parse(raw.as_bytes()).unwrap();
        let mut out = EffectSink::new();

        dispatch_ws(&mut ctx, &frame, &mut out).unwrap();

        assert!(
            out.as_slice()
                .iter()
                .any(|effect| matches!(effect, Effect::PersistFire { .. })),
            "legacy event should still apply through channel.ingest, got: {:?}",
            out.as_slice()
        );
        assert!(
            out.as_slice()
                .iter()
                .any(|effect| matches!(effect, Effect::Emit { .. })),
            "legacy event should still emit domain event, got: {:?}",
            out.as_slice()
        );
    }

    #[test]
    fn dispatch_ws_routes_hello_through_registry() {
        let mut state = ImState::new();
        let channel_id = ChannelId::from_test_u64(7);
        state
            .channels
            .insert(channel_id, Channel::new(channel_id, 42));
        state.increment_fetched.insert(channel_id);

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
            ctx.state.increment_fetched.is_empty(),
            "hello should reset the previous increment batch"
        );

        let mut saw_established = false;
        let mut saw_channels_loaded = false;
        let mut saw_increment_http = false;
        for effect in out.as_slice() {
            match effect {
                Effect::Emit { event } => {
                    let body = String::from_utf8_lossy(event.0.as_ref());
                    saw_established |=
                        body.contains("im:connection:established") && body.contains("CONN_REG");
                    saw_channels_loaded |= body.contains("im:channels:loaded");
                }
                Effect::Http { corr, req } => {
                    saw_increment_http = true;
                    assert_eq!(corr.raw(), 91);
                    assert_eq!(req.method, "POST");
                    assert!(req.url.ends_with("/channels/load/increment"));
                    assert!(
                        req.headers
                            .iter()
                            .any(|(key, value)| key == "connectionId" && value == "CONN_REG"),
                        "increment HTTP should carry connectionId header, got: {:?}",
                        req.headers
                    );
                    let body: serde_json::Value =
                        serde_json::from_slice(req.body.as_ref().unwrap()).unwrap();
                    assert_eq!(body["cursors"][0]["channelId"], channel_id.as_str());
                    assert_eq!(body["cursors"][0]["fromSeq"], Seq(42).0);
                }
                _ => {}
            }
        }
        assert!(saw_established, "missing connection established emit");
        assert!(saw_increment_http, "missing increment HTTP trigger");
        assert!(saw_channels_loaded, "missing channels loaded emit");
    }

    #[test]
    fn dispatch_ws_increment_channel_registers_channel_and_emits_increment() {
        let mut state = ImState::new();
        let mut alloc_corr = || helix_core::Correlation::from_raw(1);
        let channel_id = channel_id(77);
        let mut ctx = ImWsContext::new(&mut state, 1_000, "", &mut alloc_corr);
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
        assert_eq!(channel.cursor.value(), Seq(42));
        assert!(ctx.state.increment_fetched.contains(&channel_id));
        assert!(!ctx.state.need_sync_skip.contains(&channel_id));
        assert_eq!(ctx.state.increment_target.get(&channel_id), Some(&Seq(42)));
        assert!(
            out.as_slice().iter().filter_map(emit_body).any(|body| {
                body.contains("im:channel:increment") && body.contains(channel_id.as_str())
            }),
            "increment should emit channel increment, got: {:?}",
            out.as_slice()
        );
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
            &mut alloc_corr,
        );
        let frame = frame_from_json(serde_json::json!({
            "action": "increment_channel_end",
            "data": {}
        }));
        let mut out = EffectSink::new();

        dispatch_ws(&mut ctx, &frame, &mut out).unwrap();

        assert_eq!(sync_http_channel_ids(&out), vec![sync_channel.as_str()]);
        assert_eq!(
            ctx.state
                .corr_map
                .get(&helix_core::Correlation::from_raw(7)),
            Some(&crate::state::CorrelationContext::SyncPull {
                channel_id: sync_channel
            })
        );
        assert!(
            ctx.state
                .channels
                .get(&sync_channel)
                .unwrap()
                .inflight_sync
                .is_some(),
            "global end sync should set inflight guard"
        );
        let req = out
            .as_slice()
            .iter()
            .find_map(|effect| match effect {
                Effect::Http { req, .. } => Some(req),
                _ => None,
            })
            .expect("global end should emit sync HTTP");
        assert!(
            req.headers
                .iter()
                .any(|(key, value)| key == "connectionId" && value == "CONN_INC"),
            "sync should carry connectionId header, got: {:?}",
            req.headers
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

        let mut alloc_corr = || helix_core::Correlation::from_raw(7);
        let mut ctx = ImWsContext::new(
            &mut state,
            1_000,
            "https://example.test/api",
            &mut alloc_corr,
        );
        let frame = frame_from_json(serde_json::json!({
            "action": "increment_channel_end",
            "data": { "channelId": "not-a-valid-channel-id" }
        }));
        let mut out = EffectSink::new();

        dispatch_ws(&mut ctx, &frame, &mut out).unwrap();

        assert_eq!(sync_http_channel_ids(&out), vec![sync_channel.as_str()]);
    }

    #[test]
    fn dispatch_ws_subtopic_increment_end_emits_update_without_sync() {
        let mut state = ImState::new();
        let mut alloc_corr = || helix_core::Correlation::from_raw(1);
        let mut ctx = ImWsContext::new(
            &mut state,
            1_000,
            "https://example.test/api",
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

    #[test]
    fn dispatch_ws_invalid_increment_channel_is_silent_noop() {
        let mut state = ImState::new();
        let mut alloc_corr = || helix_core::Correlation::from_raw(1);
        let mut ctx = ImWsContext::new(&mut state, 1_000, "", &mut alloc_corr);
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
