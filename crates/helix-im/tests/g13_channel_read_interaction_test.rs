use helix_core::effect::Effect;
use helix_core::Correlation;
use helix_im::commands::handle_outbound;
use helix_im::projector::unread_summary::{
    ChannelReadObservation, ChannelReadObservationDecision, ChannelReadObservationTracker,
};
use serde_json::{json, Value};

#[path = "support/g13_scenario.rs"]
mod scenario;

/// 返回 canonical outbound command 的唯一 HTTP URL 与 body。
fn one_http(name: &str, payload: Value) -> (String, Value) {
    let effects = handle_outbound(
        name,
        &serde_json::to_vec(&payload).unwrap(),
        "http://x/api/cses",
        "http://x/api/cses",
        Some("conn-g13"),
        Correlation::from_raw(13),
    )
    .expect("canonical read command");
    match effects.as_slice() {
        [Effect::Http { req, .. }] => (
            req.url.clone(),
            serde_json::from_slice(req.body.as_ref().unwrap()).unwrap(),
        ),
        other => panic!("expected one HTTP effect, got {other:?}"),
    }
}

/// 证明稳定可见边界只推进一次 read observation。
#[test]
fn stable_visibility_advances_once_per_monotonic_boundary() {
    let mut tracker = ChannelReadObservationTracker::default();
    let first = ChannelReadObservation::new(7, "window-1", 41, true);
    assert_eq!(
        tracker.observe(first.clone()),
        ChannelReadObservationDecision::AwaitingStability
    );
    assert_eq!(
        tracker.observe(first),
        ChannelReadObservationDecision::Advance { boundary: 41 }
    );
    assert_eq!(
        tracker.observe(ChannelReadObservation::new(7, "window-1", 41, true)),
        ChannelReadObservationDecision::Noop
    );
    assert_eq!(
        tracker.observe(ChannelReadObservation::new(7, "window-1", 40, true)),
        ChannelReadObservationDecision::Noop
    );
}

/// 证明 epoch/window 改变必须重新获得两次稳定 observation。
#[test]
fn epoch_or_window_change_requires_a_new_stability_pair() {
    let mut tracker = ChannelReadObservationTracker::default();
    tracker.observe(ChannelReadObservation::new(7, "window-1", 41, true));
    assert_eq!(
        tracker.observe(ChannelReadObservation::new(8, "window-1", 42, true)),
        ChannelReadObservationDecision::AwaitingStability
    );
    assert_eq!(
        tracker.observe(ChannelReadObservation::new(8, "window-2", 42, true)),
        ChannelReadObservationDecision::AwaitingStability
    );
    assert_eq!(
        tracker.observe(ChannelReadObservation::new(8, "window-2", 42, true)),
        ChannelReadObservationDecision::Advance { boundary: 42 }
    );
}

/// 证明 channel-view wire 不携带客户端伪造的已读 authority。
#[test]
fn canonical_channel_view_wire_does_not_smuggle_read_authority() {
    let (url, body) = one_http(
        "im_channels_view",
        json!({"channels":[{"id":"channel-13","isRoot":true}]}),
    );
    assert_eq!(url, "http://x/api/cses/channels/view");
    assert_eq!(
        body,
        json!({"channels":[{"id":"channel-13","isRoot":true}]})
    );
    assert!(body.get("unreadCount").is_none());
    assert!(body.get("userId").is_none());
}

/// 证明 update_channel 的 paired Frame 只由 matching PersistOk 释放。
#[test]
fn update_channel_read_authority_obeys_persist_barrier() {
    let trace = scenario::G13ScenarioRunner::run_success();
    scenario::assert_group_created_contract_for(
        &trace.group_created,
        &scenario::g13_scenario_spec(),
    );
    assert!(trace.request.url.ends_with("/channels/view"));
    assert!(trace.persist_corr.raw() > 0);
    scenario::assert_initial_contract(&trace.initial);
    scenario::assert_terminal_contract(&trace.terminal);
}

/// 证明 PersistErr、重复 authority 与迟到 reply 都不会发布伪终态。
#[test]
fn read_authority_failures_and_races_are_idempotent() {
    let persist_failure = scenario::G13ScenarioRunner::run_persist_failure();
    let duplicate = scenario::G13ScenarioRunner::run_duplicate_authority();
    let late = scenario::G13ScenarioRunner::run_late_persist_reply();
    scenario::assert_initial_contract(&persist_failure.initial);
    scenario::assert_initial_contract(&duplicate.initial);
    scenario::assert_initial_contract(&late.initial);
    assert_eq!(persist_failure.frame_count, 0);
    assert_eq!(duplicate.frame_count, 0);
    assert_eq!(late.frame_count, 0);
}

/// 证明 durable row 可在 cold attach 时重建 paired typed snapshot。
#[test]
fn read_state_recovers_from_durable_dialog_row() {
    let frame = scenario::G13ScenarioRunner::run_recovery();
    frame.validate().expect("G13 recovered Frame validates");
    assert_eq!(frame.mutations.len(), 2);
}
