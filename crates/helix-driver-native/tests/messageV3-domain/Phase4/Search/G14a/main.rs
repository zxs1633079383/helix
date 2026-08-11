use crate::shared;

/// MV3-G14a：真实 global/grouped search 读取，核对 Go 返回的 messageGroups 与 seeded post。
#[tokio::test]
async fn mv3_g14a_real_chain() {
    shared::assert_search_real_chain(
        "Phase4",
        "Search",
        "MV3-G14a",
        "im_search_do",
        "/api/cses/search/do",
        |_channel, marker, _post_id| serde_json::json!({"keyword": marker, "page_size": 20}),
    )
    .await
    .unwrap_or_else(|error| panic!("MV3-G14a FAIL: {error}"));
}
