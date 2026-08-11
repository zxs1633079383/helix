use crate::shared;

/// MV3-G14b：真实探测频道搜索现有路由，并记录目标命令未注册的合同缺口。
#[tokio::test]
async fn mv3_g14b_real_chain() {
    shared::assert_search_contract_gap(
        "Phase4",
        "Search",
        "MV3-G14b",
        "im_search_in_channel",
        "im_search_post",
        "/api/cses/search/post",
        |channel| serde_json::json!({"req_id": shared::run_token("search-channel"), "keyword": shared::resource_name("search"), "channel_id": channel, "scope": "channel", "page_size": 20}),
    )
    .await
    .unwrap_or_else(|error| panic!("MV3-G14b FAIL: {error}"));
}
