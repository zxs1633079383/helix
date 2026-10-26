use crate::shared;

/// MV3-G15c：真实 mention post 后查询待办，核对 mention 类型与删除权限。
#[tokio::test]
async fn mv3_g15c_real_chain() {
    shared::assert_todo_real_probe("Phase4", "Todo", "MV3-G15c", false, false, "mention", true)
        .await
        .unwrap_or_else(|error| panic!("MV3-G15c FAIL: {error}"));
}
