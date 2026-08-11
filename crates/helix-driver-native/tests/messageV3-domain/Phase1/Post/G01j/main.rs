use crate::shared;

/// MV3-G01j：保持真实前置缺口可见，不用 fake server 或 HTTP 200 伪造通过。
#[tokio::test]
#[ignore = "NOT_VERIFIED/NOT_APPLICABLE: run explicitly after the named precondition is available"]
async fn mv3_g01j_real_chain() {
    shared::record_unverified("Phase1", "Post", "MV3-G01j", "Angular G01j is AVERAGE_SCORE contract-only; current real Go fixture and terminal projection are not verified.").unwrap();
}
