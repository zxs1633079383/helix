use crate::shared;

/// MV3-G03a：按 Angular item-forward wire 创建真实 posts，并核对目标频道 WS。
#[tokio::test]
async fn mv3_g03a_real_chain() {
    shared::assert_forward_chain("Phase1", "Post", "MV3-G03a", 1)
        .await
        .unwrap_or_else(|error| panic!("MV3-G03a FAIL: {error}"));
}
