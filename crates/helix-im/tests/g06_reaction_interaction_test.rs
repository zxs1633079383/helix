//! G06 focused contract: reaction intent is opaque and the Go wire stays postId/emoji only.

use helix_core::effect::Effect;
use helix_core::Correlation;
use helix_im::commands::handle_outbound;
use helix_im::interaction_frame::TypedSurfaceMutation;
use helix_im::projector::timeline::{TimelineProjector, TimelineWindowRequest};
use serde_json::json;

#[path = "support/g06_scenario.rs"]
mod scenario;

use scenario::{
    assert_reactable_contract, assert_reacted_contract, g06_scenario_spec, G06ScenarioRunner,
};

#[test]
/// 证明投影器只签发 opaque react binding，不把业务 command 暴露给平台。
fn reaction_capability_produces_an_opaque_live_binding() {
    let mut projector = TimelineProjector::default();
    let mutation = projector
        .snapshot_from_render_ready(
            TimelineWindowRequest::latest("channel-reaction"),
            &[json!({
                "id": "post-reaction", "channelId": "channel-reaction", "userId": "444",
                "createdAt": 1, "type": "TEXT", "message": "react",
                "sendStatus": "sent", "props": {},
                "actionCapabilities": { "canReact": true }
            })],
        )
        .expect("timeline fixture must project");
    let actions = match mutation.mutation {
        TypedSurfaceMutation::TimelineSnapshot { view } => view.items[0].actions.clone(),
        other => panic!("expected timeline snapshot, got {other:?}"),
    };
    assert!(
        actions
            .iter()
            .find(|action| action.kind == "react")
            .expect("react binding")
            .enabled
    );
}

#[test]
/// 证明 canonical direct command 只发送 postId/emoji 并拒绝本地 desiredSelected。
fn quick_reply_wire_rejects_desired_selected_and_contains_only_post_id_and_emoji() {
    let effects = handle_outbound(
        "im_quick_reply",
        &serde_json::to_vec(&json!({ "post_id": "p1", "emoji": "👍", "req_id": "reaction-1" }))
            .unwrap(),
        "http://test/api/cses",
        "http://test/api/cses",
        None,
        Correlation::from_raw(1),
    )
    .expect("canonical reaction command");
    let body = match &effects[0] {
        Effect::Http { req, .. } => {
            serde_json::from_slice::<serde_json::Value>(req.body.as_ref().expect("body")).unwrap()
        }
        other => panic!("expected HTTP, got {other:?}"),
    };
    assert_eq!(body, json!({ "postId": "p1", "emoji": "👍" }));

    let invalid = handle_outbound(
        "im_quick_reply",
        &serde_json::to_vec(&json!({ "post_id": "p1", "emoji": "👍", "desired_selected": true }))
            .unwrap(),
        "http://test/api/cses",
        "http://test/api/cses",
        None,
        Correlation::from_raw(2),
    );
    assert!(
        invalid.is_err(),
        "desiredSelected is local intent and must not cross the Go wire"
    );
}

#[test]
/// 证明 reaction 仅在 WS authority 持久化后携 action causation 发布。
fn reaction_waits_for_persist_barrier_and_preserves_action_causation() {
    let trace = G06ScenarioRunner::run_ws_reaction();
    assert_reactable_contract(&trace.reactable);
    assert_reacted_contract(&trace.reacted, Some(g06_scenario_spec().request_id));
}

#[test]
/// 证明持久化失败后的 authority 重放收敛为同一字段级 reaction。
fn reaction_authority_replays_after_persist_failure() {
    let trace = G06ScenarioRunner::run_persist_failure_replay();
    assert_reacted_contract(
        &trace.reacted_after_replay,
        Some(g06_scenario_spec().request_id),
    );
}

#[test]
/// 证明 durable row 可在无平台 reducer 时恢复完整 reaction snapshot。
fn reaction_recovery_preserves_typed_frame_fields() {
    let trace = G06ScenarioRunner::run_recovery();
    assert_reacted_contract(&trace.recovered, None);
}
