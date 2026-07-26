//! G12 focused contract: reply hydration is request-scoped and relation-safe.

#[path = "support/g12_scenario.rs"]
mod scenario;

use helix_im::render_ready_replies::{
    extract_flat_replies, finalize_projection, ReplyProjectionMode, ReplyProjectionRequest,
};
use serde_json::json;

/// 构造 reply normalization 单元测试使用的稳定 request。
fn request() -> ReplyProjectionRequest {
    ReplyProjectionRequest {
        req_id: "reply-12".into(),
        channel_id: "channel-12".into(),
        root_hint: "root-12".into(),
        mode: ReplyProjectionMode::Snapshot,
        revision: 12,
        viewer_user_id: "444".into(),
    }
}

/// 证明同 channel reply graph 被接受，而 mixed authority 整批清空。
#[test]
fn reply_page_accepts_matching_graph_and_rejects_mixed_authority_batch() {
    let matching = extract_flat_replies(
        &json!({
            "rootPost": {"id":"root-12","channelId":"channel-12","userId":"444"},
            "replies": [{"id":"reply-12-1","channelId":"channel-12","userId":"678","replyId":"root-12"}]
        }),
        "444",
    );
    let matching = finalize_projection(&request(), matching);
    assert_eq!(matching.reply_ids, vec!["reply-12-1"]);
    assert_eq!(matching.reply_count, 1);

    let mixed = extract_flat_replies(
        &json!({
            "rootPost": {"id":"root-12","channelId":"channel-12","userId":"444"},
            "replies": [{"id":"cross","channelId":"other","userId":"678","replyId":"root-12"}]
        }),
        "444",
    );
    let rejected = finalize_projection(&request(), mixed);
    assert!(rejected.nodes.is_empty());
    assert!(rejected.reply_ids.is_empty());
}

/// 证明读-only G12 终态不虚构 Persist barrier，且只输出一个完整 Frame。
#[test]
fn reply_terminal_is_frame_only_without_raw_or_persist_side_channel() {
    let trace = scenario::G12ScenarioRunner::run_success();
    assert_eq!(trace.snapshot_effect_count, 1);
    assert_eq!(trace.snapshot_raw_emit_count, 0);
    assert_eq!(trace.snapshot_persist_count, 0);
    scenario::assert_snapshot_contract(&trace.snapshot);
    scenario::assert_branch_contract(&trace.branch);
}

/// 证明 late duplicate 与 detached recovery 都由 correlation/projector 决定。
#[test]
fn duplicate_and_detached_recovery_remain_idempotent() {
    let duplicate = scenario::G12ScenarioRunner::run_duplicate_reply();
    assert_eq!(duplicate.frame_count, 0);
    let recovery = scenario::G12ScenarioRunner::run_detached_recovery();
    assert_eq!(recovery.frame_count, 0);
    scenario::assert_snapshot_contract(
        recovery
            .recovered
            .as_ref()
            .expect("detached thread recovered from snapshot"),
    );
}
