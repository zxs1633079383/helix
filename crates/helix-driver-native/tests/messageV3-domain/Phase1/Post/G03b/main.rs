use crate::shared;

/// MV3-G03b：按 Angular merge-forward wire 创建真实 MULTIPLY post，并核对 WS。
#[tokio::test]
async fn mv3_g03b_real_chain() {
    shared::assert_forward_chain("Phase1", "Post", "MV3-G03b", 2)
        .await
        .unwrap_or_else(|error| panic!("MV3-G03b FAIL: {error}"));
}
