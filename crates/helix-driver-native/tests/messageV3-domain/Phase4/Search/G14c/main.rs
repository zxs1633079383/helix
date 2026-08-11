use crate::shared;

/// MV3-G14c：真实探测搜索回传，并记录 grouped message 目标未注册的合同缺口。
#[tokio::test]
async fn mv3_g14c_real_chain() {
    shared::assert_search_contract_gap(
        "Phase4",
        "Search",
        "MV3-G14c",
        "im_search_messages_grouped",
        "im_search_do",
        "/api/cses/search/do",
        |_| serde_json::json!({"req_id": shared::run_token("search-grouped"), "keyword": shared::resource_name("search-grouped")}),
    )
    .await
    .unwrap_or_else(|error| panic!("MV3-G14c FAIL: {error}"));
}
