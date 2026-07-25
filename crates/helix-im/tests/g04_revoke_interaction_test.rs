//! G04/G04a focused contract: revoke capability and offline authority stay behind persistence.

mod common;
use common::*;

use helix_core::tick::InboundBytes;
use helix_core::{Effect, Tick};
use helix_im::interaction_frame::TypedSurfaceMutation;
use helix_im::projector::timeline::{TimelineProjector, TimelineWindowRequest};
use serde_json::json;

#[path = "support/g04_scenario.rs"]
mod scenario;

#[path = "support/g04a_scenario.rs"]
mod offline_scenario;

/// 从一条 render-ready row 提取 Helix 签发的 timeline actions。
fn timeline_actions(
    capabilities: serde_json::Value,
) -> Vec<helix_im::interaction_frame::ActionBinding> {
    let channel_id = cid(4);
    let mut projector = TimelineProjector::default();
    let mutation = projector
        .snapshot_from_render_ready(
            TimelineWindowRequest::latest(channel_id.clone()),
            &[json!({
                "id": sid(4), "channelId": channel_id, "userId": "444",
                "createdAt": 1, "type": "TEXT", "message": "revoke me",
                "sendStatus": "sent", "props": {}, "actionCapabilities": capabilities
            })],
        )
        .expect("timeline fixture must project");
    match mutation.mutation {
        TypedSurfaceMutation::TimelineSnapshot { view } => view.items[0].actions.clone(),
        other => panic!("expected timeline snapshot, got {other:?}"),
    }
}

#[test]
/// 证明只有 authority capability 才会生成可执行 opaque revoke binding。
fn revoke_capability_is_live_only_when_authority_grants_it() {
    let actions = timeline_actions(json!({ "canRevoke": true }));
    let revoke = actions
        .iter()
        .find(|action| action.kind == "revoke")
        .expect("revoke binding");
    assert!(
        revoke.enabled,
        "TARGET: trusted canRevoke must produce a live opaque binding"
    );
    assert!(revoke.disabled_reason.is_none());
}

#[test]
/// 证明离线撤回 fact 在 PersistAtomic 成功前不发布业务 projection。
fn offline_revoke_fact_waits_for_one_atomic_persist_before_business_projection() {
    let channel_id = cid(4);
    let message_id = sid(4);
    let mut harness = Harness::new_with_identity(&[4], "678", "team-1", "", "", "");
    harness.step(
        Tick::Inbound(InboundBytes(
            serde_json::to_vec(&json!({
                "action": "posts_update",
                "data": {
                    "event_seq": 1,
                    "channelId": channel_id,
                    "posts": [{
                        "id": message_id,
                        "channelId": cid(4),
                        "userId": "444",
                        "revoke": true,
                        "message": "recalled"
                    }]
                }
            }))
            .unwrap()
            .into(),
        )),
        10,
    );

    assert!(
        !harness
            .sink
            .as_slice()
            .iter()
            .any(|effect| matches!(effect, Effect::Http { .. })),
        "G04a offline replay must never create an online revoke HTTP request"
    );
    assert_eq!(
        harness
            .sink
            .as_slice()
            .iter()
            .filter(|effect| matches!(effect, Effect::PersistAtomic { .. }))
            .count(),
        1,
        "message revoke, viewer indexes and eventSeq must share one atomic write-set"
    );
    assert!(
        !harness
            .sink
            .as_slice()
            .iter()
            .any(|effect| matches!(effect, Effect::Emit { .. })),
        "business projection must wait for the matching PersistOk"
    );
}

#[test]
/// 证明在线 action 的 WS authority 在 PersistOk 后才产生 recalled Frame。
fn online_revoke_waits_for_authority_persist_and_preserves_causation() {
    let trace = scenario::G04ScenarioRunner::run_ws_revoke();
    scenario::assert_revocable_contract(&trace.revocable);
    scenario::assert_recalled_contract(
        &trace.recalled,
        Some(scenario::g04_scenario_spec().request_id),
    );
}

#[test]
/// 证明撤回持久失败可重放且不会先发布部分 recalled 状态。
fn online_revoke_persist_failure_replays_without_partial_frame() {
    let trace = scenario::G04ScenarioRunner::run_persist_failure_replay();
    scenario::assert_recalled_contract(
        &trace.recalled_after_replay,
        Some(scenario::g04_scenario_spec().request_id),
    );
}

#[test]
/// 证明 G04a 公开 sync command 只采用 Helix cursor，提交后 Frame 不继承平台 causation。
fn offline_revoke_uses_helix_cursor_and_emits_authority_frame_after_commit() {
    let trace = offline_scenario::G04AScenarioRunner::run_offline_revoke();
    assert_eq!(trace.submitted_from_seq, 0);
    offline_scenario::assert_revocable_contract(&trace.revocable);
    offline_scenario::assert_offline_recalled_contract(&trace.recalled);
}
