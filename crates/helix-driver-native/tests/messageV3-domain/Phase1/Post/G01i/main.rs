use crate::shared;
use serde_json::json;

/// MV3-G01i：通过 Helix `im_vote_create` 调 Java，再以 Go/WS VOTE 回声验收。
#[tokio::test]
async fn mv3_g01i_real_chain() {
    let title = shared::resource_name("vote");
    shared::assert_java_engagement_create_chain(
        "Phase1",
        "Post",
        "MV3-G01I",
        "im_vote_create",
        "/vote/createVote",
        json!({
            "title": title.clone(),
            "content": "Phase1 real-chain vote",
            "votes": 1,
            "is_real": false,
            "finish_time": 4_000_000_000_000_i64,
            "options": [
                {"index": "1", "value": "通过", "approval_count": 0},
                {"index": "2", "value": "驳回", "approval_count": 0}
            ],
            "join_count": 0
        }),
        "VOTE",
        &title,
        "vote",
    )
    .await
    .unwrap_or_else(|error| panic!("MV3-G01I FAIL: {error}"));
}
