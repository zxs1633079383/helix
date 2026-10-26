use crate::shared;

/// MV3-G14c：真实无频道搜索，核对 Go grouped message page 与 seeded post identity。
#[tokio::test]
async fn mv3_g14c_real_chain() {
    shared::assert_search_real_chain(
        "Phase4",
        "Search",
        "MV3-G14c",
        "im_search_do",
        "/api/cses/search/do",
        |_channel, marker, _post_id| serde_json::json!({"keyword": marker, "page_size": 20}),
    )
    .await
    .unwrap_or_else(|error| panic!("MV3-G14c FAIL: {error}"));
}
