use crate::shared;

/// MV3-G14b：真实频道范围搜索，核对结果通过 Helix read relay 返回且命中目标 post。
#[tokio::test]
async fn mv3_g14b_real_chain() {
    shared::assert_search_real_chain(
        "Phase4",
        "Search",
        "MV3-G14b",
        "im_search_post",
        "/api/cses/search/post",
        |channel, marker, _post_id| serde_json::json!({"keyword": marker, "channel_id": channel, "page_size": 20}),
    )
    .await
    .unwrap_or_else(|error| panic!("MV3-G14b FAIL: {error}"));
}
