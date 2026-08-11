use crate::shared;

/// MV3-G03b：保留 Angular merge-forward 的真实前置缺口，不用错误 wire 伪造通过。
#[tokio::test]
#[ignore = "NOT_VERIFIED: Angular merge forward requires MULTIPLY mergeMessage assembled from local authority rows"]
async fn mv3_g03b_real_chain() {
    shared::record_unverified(
        "Phase1",
        "Post",
        "MV3-G03b",
        "Angular merge mode requires source channel title and MULTIPLY mergeMessage authority; current target has no verified full projection fixture",
    )
    .unwrap();
}
