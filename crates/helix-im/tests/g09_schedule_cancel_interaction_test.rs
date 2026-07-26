//! G09 focused contract: canceled authority clears draft content; delivery remains G01-owned.

use helix_core::Effect;
use helix_im::acl::to_effect_s1::emit_schedule_canceled;
use helix_im::state::ChannelId;
use serde_json::json;

#[path = "support/g09_scenario.rs"]
mod scenario;

use scenario::{
    assert_canceled_summary_contract, assert_scheduled_prerequisite_contract, G09ScenarioRunner,
    REQUEST_ID,
};

/// 证明 canceled domain event 不携带已退役正文或 G01 delivery 字段。
#[test]
fn cancel_terminal_projection_cannot_resurrect_retired_schedule_body() {
    let channel_id = ChannelId::from_str("chfixx00000000000000000009").expect("valid test id");
    let Effect::Emit { event } = emit_schedule_canceled(
        channel_id,
        &json!({
            "channelId": "chfixx00000000000000000009",
            "message": "must disappear",
            "messagePreview": "must disappear",
            "revision": 9
        }),
        None,
        true,
    ) else {
        panic!("schedule cancel must emit terminal projection")
    };
    let payload: serde_json::Value = serde_json::from_slice(event.0.as_ref()).unwrap();
    assert_eq!(payload["data"]["scheduleStatus"], "canceled");
    assert_eq!(payload["data"]["hasSchedulePost"], false);
    assert!(payload["data"]["scheduleMessage"].is_null());
    assert!(payload["data"]["scheduleMessagePreview"].is_null());
    assert!(payload["data"].get("deliveredMessage").is_none());
}

/// 证明完整 cancel 交互只在 authority 持久成功后替换已排程摘要。
#[test]
fn cancel_authority_replaces_scheduled_summary_after_persist_barrier() {
    let trace = G09ScenarioRunner::run_canceled();
    assert_scheduled_prerequisite_contract(&trace.scheduled_summary);
    assert_canceled_summary_contract(&trace.canceled_summary, Some(REQUEST_ID));
}

/// 证明 PersistAtomic 失败不泄漏 canceled Frame，重放后只提交一次终态。
#[test]
fn cancel_persist_failure_replays_without_partial_or_duplicate_frame() {
    let trace = G09ScenarioRunner::run_persist_failure_replay();
    assert_scheduled_prerequisite_contract(&trace.scheduled_summary);
    assert_canceled_summary_contract(&trace.canceled_after_replay, Some(REQUEST_ID));
}
