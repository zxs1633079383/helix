use crate::shared;

/// MV3-G02c：保持真实前置缺口可见，不用 fake server 或 HTTP 200 伪造通过。
#[tokio::test]
#[ignore = "NOT_VERIFIED/NOT_APPLICABLE: run explicitly after the named precondition is available"]
async fn mv3_g02c_real_chain() {
    shared::record_unverified("Phase1", "Post", "MV3-G02c", "retry needs a durable failed row and an approved real retry fixture; no synthetic retry is allowed.").unwrap();
}
