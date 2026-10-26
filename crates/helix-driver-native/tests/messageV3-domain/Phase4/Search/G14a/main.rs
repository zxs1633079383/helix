use crate::shared;

/// MV3-G14a：真实全局搜索三段命令，核对会话、联系人、消息的 Go DTO 与 read relay。
#[tokio::test]
async fn mv3_g14a_real_chain() {
    shared::assert_search_global_real_chain("Phase4", "Search", "MV3-G14a")
        .await
        .unwrap_or_else(|error| panic!("MV3-G14a FAIL: {error}"));
}
