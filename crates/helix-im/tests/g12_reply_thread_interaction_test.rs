//! G12 focused contract: reply hydration is request-scoped and relation-safe.

use helix_im::render_ready_replies::{
    extract_flat_replies, finalize_projection, ReplyProjectionMode, ReplyProjectionRequest,
};
use serde_json::json;

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
