use crate::shared;

/// MV3-G15b：真实 urgent post/confirm 后查询待办，核对确认态投影。
#[tokio::test]
async fn mv3_g15b_real_chain() {
    shared::assert_todo_real_probe("Phase4", "Todo", "MV3-G15b", true, true, "urgent", false)
        .await
        .unwrap_or_else(|error| panic!("MV3-G15b FAIL: {error}"));
}
