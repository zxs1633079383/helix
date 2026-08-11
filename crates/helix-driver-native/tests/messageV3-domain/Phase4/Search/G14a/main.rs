use crate::shared;

/// MV3-G14a：真实探测现有搜索链，并将目标 global/grouped 命令缺口写入证据。
#[tokio::test]
async fn mv3_g14a_real_chain() {
    shared::assert_search_contract_gap(
        "Phase4",
        "Search",
        "MV3-G14a",
        "im_search_global / im_search_messages_grouped",
        "im_search_do",
        "/api/cses/search/do",
        |channel| serde_json::json!({"req_id": shared::run_token("search-global"), "keyword": channel}),
    )
    .await
    .unwrap_or_else(|error| panic!("MV3-G14a FAIL: {error}"));
}
