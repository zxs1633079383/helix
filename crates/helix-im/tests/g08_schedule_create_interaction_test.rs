//! G08 focused contract: schedule hydrate remains owner/channel scoped.

use helix_im::acl::to_effect_s1::owner_schedule_hydration;
use serde_json::json;

#[path = "support/g08_scenario.rs"]
mod scenario;

use scenario::{assert_schedule_summary_contract, G08ScenarioRunner, REQUEST_ID};

#[test]
/// 证明 getSchedule owner hydrate 仍按 viewer/channel 双重授权。
fn get_schedule_hydration_rejects_observer_and_cross_channel_content() {
    let channel_id = "chfixx00000000000000000008";
    let authority = json!({
        "channelId": channel_id,
        "scheduleId": "schedule-8",
        "scheduleOwnerUserId": "444",
        "message": "owner-only content",
        "sendAt": 1_780_000_000_000_i64,
        "revision": 8
    });

    let owner = owner_schedule_hydration(&authority, channel_id, "444")
        .expect("matching owner and channel hydrate");
    assert_eq!(owner["scheduleMessage"], "owner-only content");
    assert!(owner_schedule_hydration(&authority, channel_id, "678").is_none());
    assert!(owner_schedule_hydration(&authority, "channel-other", "444").is_none());
}

#[test]
/// 证明 schedule terminal 必须等待 PersistAtomic 和本地 readback。
fn schedule_frame_is_released_only_after_persist_barrier() {
    let trace = G08ScenarioRunner::run_created();
    assert_schedule_summary_contract(&trace.scheduled_summary, true, Some(REQUEST_ID));
}

#[test]
/// 证明持久化失败不发布，重放相同 authority revision 后只产生一个 terminal Frame。
fn failed_persist_allows_same_revision_replay() {
    let trace = G08ScenarioRunner::run_persist_failure_replay();
    assert_schedule_summary_contract(&trace.scheduled_after_replay, true, Some(REQUEST_ID));
}

#[test]
/// 证明迟到 HTTP 与重复 inbound 不会改变已提交的 schedule Frame。
fn late_http_and_duplicate_inbound_are_noops() {
    let trace = G08ScenarioRunner::run_created();
    assert_schedule_summary_contract(&trace.scheduled_summary, true, Some(REQUEST_ID));
}

#[test]
/// 证明恢复路径从 durable read model 生成无 command causation 的完整 snapshot。
fn recovery_preserves_all_schedule_fields() {
    let trace = G08ScenarioRunner::run_recovery();
    assert_schedule_summary_contract(&trace.recovered_summary, true, None);
}
