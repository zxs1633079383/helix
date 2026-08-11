use crate::shared;

/// MV3-G15a：真实 urgent post 后查询待办，核对 urgent todo projection。
#[tokio::test]
async fn mv3_g15a_real_chain() {
    shared::assert_todo_real_probe("Phase4", "Todo", "MV3-G15a", true, false, "urgent", false)
        .await
        .unwrap_or_else(|error| panic!("MV3-G15a FAIL: {error}"));
}
