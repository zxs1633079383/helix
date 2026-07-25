//! G05 focused contract: forward binding, deterministic assembly and per-target partial terminal.

mod common;
use common::*;

use helix_core::Effect;
use helix_im::interaction_frame::{InteractionFrameV2, SurfaceId, TypedSurfaceMutation};
use helix_im::projector::timeline::{TimelineProjector, TimelineWindowRequest};
use serde_json::json;

#[test]
/// 证明 TimelineProjector 仅以 opaque binding 暴露 Helix 签发的转发权限。
fn forward_capability_produces_an_opaque_live_binding() {
    let channel_id = cid(5);
    let mut projector = TimelineProjector::default();
    let mutation = projector
        .snapshot_from_render_ready(
            TimelineWindowRequest::latest(channel_id.clone()),
            &[json!({
                "id": sid(5), "channelId": channel_id, "userId": "444",
                "createdAt": 1, "type": "TEXT", "message": "forward me",
                "sendStatus": "sent", "props": {},
                "actionCapabilities": { "canForward": true }
            })],
        )
        .expect("timeline fixture must project");
    let actions = match mutation.mutation {
        TypedSurfaceMutation::TimelineSnapshot { view } => view.items[0].actions.clone(),
        other => panic!("expected timeline snapshot, got {other:?}"),
    };
    let forward = actions
        .iter()
        .find(|action| action.kind == "forward")
        .expect("forward binding");
    assert!(
        forward.enabled,
        "TARGET: canForward must be invokable through its opaque actionRef"
    );
}

#[test]
/// 证明 per-target HTTP admission 只报告 pending/failed，不声称权威送达。
fn per_target_partial_acceptance_never_claims_all_success_or_delivery() {
    let mut harness = Harness::new(&[]);
    harness.step(
        command_json(
            "im_create_posts",
            json!({
                "req_id": "forward-partial-1",
                "posts": [{"id": ""}],
                "channel_ids": [cid(50), cid(51)]
            }),
        ),
        1,
    );
    let corr = harness
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Http { corr, .. } => Some(corr.raw()),
            _ => None,
        })
        .expect("createPosts HTTP correlation");
    harness.step(
        port_reply_with_body(
            corr,
            enveloped(
                serde_json::to_vec(&json!({
                    "status": "SUCCESS",
                    "data": { "targets": [
                        {"channelId": cid(50), "status": "accepted"},
                        {"channelId": cid(51), "status": "failed", "error": "forbidden"}
                    ]}
                }))
                .unwrap()
                .into(),
            ),
        ),
        2,
    );
    let data = harness
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Emit { event } => {
                serde_json::from_slice::<serde_json::Value>(event.0.as_ref()).ok()
            }
            _ => None,
        })
        .filter(|event| event["event"] == "im:posts:batch-result")
        .expect("batch result");
    assert_eq!(data["data"]["batchStatus"], "partial");
    assert_eq!(data["data"]["acceptedCount"], 1);
    assert_eq!(data["data"]["failedCount"], 1);
    assert_eq!(data["data"]["targets"][0]["deliveryStatus"], "pending");
}

#[test]
/// 证明只有可信顶层 tracing 且 dialog 持久读回成功才确认最终 target。
fn trusted_target_post_completes_only_after_dialog_persist_readback_frame() {
    let target_ok = cid(50);
    let target_failed = cid(51);
    let request_id = "forward-authority-1";
    let source_id = sid(5);
    let delivered_id = sid(50);
    let mut harness = Harness::new_with_identity(&[50, 51], "444", "team-1", "", "", "");

    harness.step(command_json("im_query_dialog_list", json!({})), 1);
    let dialog_scan_corr = harness
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Persist { corr, .. } => Some(corr.raw()),
            _ => None,
        })
        .expect("dialog list scan correlation");
    harness.step(
        port_reply_with_body(
            dialog_scan_corr,
            serde_json::to_vec(&json!([{
                "id": target_ok,
                "team_id": "team-1",
                "display_name": "Target",
                "type": "P",
                "last_post_at": 1,
                "last_post": "before",
                "unread_count": 0,
                "mention_count": 0,
                "urgent_count": 0
            }]))
            .unwrap()
            .into(),
        ),
        2,
    );

    harness.step(
        command_json(
            "im_create_posts",
            json!({
                "post_ids": [source_id],
                "target_channel_ids": [target_ok, target_failed],
                "mode": "individual",
                "req_id": request_id
            }),
        ),
        3,
    );
    let source_lookup_corr = harness
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Persist { corr, .. } => Some(corr.raw()),
            _ => None,
        })
        .expect("forward source lookup correlation");
    harness.step(
        port_reply_with_body(
            source_lookup_corr,
            serde_json::to_vec(&json!([{
                "id": source_id,
                "message": "forward me",
                "simple_message": "forward me",
                "type": "TEXT",
                "props": "{}",
                "mentions": "[]"
            }]))
            .unwrap()
            .into(),
        ),
        4,
    );
    let http_corr = harness
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Http { corr, req } => {
                assert!(req
                    .headers
                    .iter()
                    .any(|(name, value)| name == "Cses-Track-Id" && value == request_id));
                Some(corr.raw())
            }
            _ => None,
        })
        .expect("createPosts HTTP correlation");
    harness.step(
        port_reply_with_body(
            http_corr,
            enveloped(
                serde_json::to_vec(&json!({
                    "status": "SUCCESS",
                    "data": { "targets": [
                        {"channelId": target_ok, "status": "accepted"},
                        {"channelId": target_failed, "status": "failed", "error": "forbidden"}
                    ]}
                }))
                .unwrap()
                .into(),
            ),
        ),
        5,
    );
    assert!(
        harness.sink.as_slice().iter().all(|effect| {
            let Effect::Emit { event } = effect else {
                return true;
            };
            let value: serde_json::Value = serde_json::from_slice(event.0.as_ref()).unwrap();
            value["event"] != "im:interaction:frame"
        }),
        "HTTP acceptance must not publish a business completion Frame"
    );

    // A post payload may contain an attacker-controlled lookalike, but only top-level envelope
    // tracing is trusted for action causation.
    harness.step(
        helix_core::Tick::Inbound(helix_core::tick::InboundBytes(
            serde_json::to_vec(&json!({
                "action": "post",
                "data": {
                    "event_seq": 1,
                    "channelId": target_ok,
                    "post": {
                        "id": sid(49),
                        "channelId": target_ok,
                        "userId": "444",
                        "createAt": 9,
                        "type": "TEXT",
                        "message": "unrelated",
                        "simpleMessage": "unrelated",
                        "viewers": ["all"],
                        "props": {"csesTrackId": request_id}
                    }
                }
            }))
            .unwrap()
            .into(),
        )),
        6,
    );
    let forged_persist_corr = harness
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Persist { corr, .. } => Some(corr.raw()),
            _ => None,
        })
        .expect("unrelated post still updates its ordinary dialog projection");
    harness.step(
        port_reply_with_body(
            forged_persist_corr,
            serde_json::to_vec(&json!([{
                "unread_count": 0,
                "mention_count": 0,
                "urgent_count": 0,
                "last_root_post_at": 9
            }]))
            .unwrap()
            .into(),
        ),
        7,
    );
    let forged_frame = harness
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Emit { event } => {
                let value: serde_json::Value = serde_json::from_slice(event.0.as_ref()).ok()?;
                (value["event"] == "im:interaction:frame")
                    .then(|| serde_json::to_vec(&value["data"]).ok())
                    .flatten()
                    .and_then(|bytes| InteractionFrameV2::decode(&bytes).ok())
            }
            _ => None,
        })
        .expect("ordinary post still produces its dialog Frame");
    assert!(
        helix_im::projector::action::matching_frame_confirmation(&forged_frame, request_id,)
            .unwrap()
            .is_none(),
        "post props cannot forge forward completion"
    );

    harness.step(
        helix_core::Tick::Inbound(helix_core::tick::InboundBytes(
            serde_json::to_vec(&json!({
                "action": "post",
                "tracing": {"csesTrackId": request_id},
                "data": {
                    "event_seq": 1,
                    "channelId": target_ok,
                    "post": {
                        "id": delivered_id,
                        "channelId": target_ok,
                        "userId": "444",
                        "createAt": 10,
                        "type": "TEXT",
                        "message": "forward me",
                        "simpleMessage": "forward me",
                        "viewers": ["all"],
                        "props": {}
                    }
                }
            }))
            .unwrap()
            .into(),
        )),
        8,
    );
    let failed_persist_corr = harness
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Persist { corr, .. } => Some(corr.raw()),
            _ => None,
        })
        .expect("trusted duplicate target authority schedules a correlated readback Persist");
    assert!(
        harness.sink.as_slice().iter().all(|effect| {
            let Effect::Emit { event } = effect else {
                return true;
            };
            let value: serde_json::Value = serde_json::from_slice(event.0.as_ref()).unwrap();
            value["event"] != "im:interaction:frame"
        }),
        "trusted WS authority still waits for the correlated PersistOk"
    );
    harness.step(port_reply_err(failed_persist_corr), 9);
    assert!(
        harness.sink.as_slice().iter().all(|effect| {
            !matches!(effect, Effect::Emit { event } if {
                let value: serde_json::Value = serde_json::from_slice(event.0.as_ref()).unwrap();
                value["event"] == "im:interaction:frame"
            })
        }),
        "PersistErr must not confirm the forward target"
    );

    // A later distinct authoritative post for the same accepted target remains eligible. The
    // failed duplicate readback did not consume the request ledger.
    harness.step(
        helix_core::Tick::Inbound(helix_core::tick::InboundBytes(
            serde_json::to_vec(&json!({
                "action": "post",
                "tracing": {"csesTrackId": request_id},
                "data": {
                    "event_seq": 2,
                    "channelId": target_ok,
                    "post": {
                        "id": sid(52), "channelId": target_ok, "userId": "444",
                        "createAt": 11, "type": "TEXT", "message": "forward me",
                        "simpleMessage": "forward me", "viewers": ["all"], "props": {}
                    }
                }
            }))
            .unwrap()
            .into(),
        )),
        10,
    );
    let dialog_persist_corr = harness
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Persist { corr, .. } => Some(corr.raw()),
            _ => None,
        })
        .expect("later target authority schedules another correlated Persist");
    harness.step(
        port_reply_with_body(
            dialog_persist_corr,
            serde_json::to_vec(&json!([{
                "unread_count": 0,
                "mention_count": 0,
                "urgent_count": 0,
                "last_root_post_at": 11
            }]))
            .unwrap()
            .into(),
        ),
        11,
    );
    let frame = harness
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Emit { event } => {
                let value: serde_json::Value = serde_json::from_slice(event.0.as_ref()).ok()?;
                (value["event"] == "im:interaction:frame")
                    .then(|| serde_json::to_vec(&value["data"]).ok())
                    .flatten()
                    .and_then(|bytes| InteractionFrameV2::decode(&bytes).ok())
            }
            _ => None,
        })
        .expect("PersistOk readback must publish a typed Frame");
    let confirmation = helix_im::projector::action::matching_frame_confirmation(&frame, request_id)
        .expect("frame remains valid")
        .expect("matching causation confirms the forward target");
    assert!(confirmation
        .mutations
        .iter()
        .any(|mutation| mutation.surface == SurfaceId::MessageLeftDialogList));
}
