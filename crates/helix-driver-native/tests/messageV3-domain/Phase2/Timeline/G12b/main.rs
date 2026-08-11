use crate::shared;

/// MV3-G12b：保持真实前置缺口可见，不用 fake server 或 HTTP 200 伪造通过。
#[tokio::test]
#[ignore = "NOT_VERIFIED/NOT_APPLICABLE: run explicitly after the named precondition is available"]
async fn mv3_g12b_real_chain() {
    shared::record_unverified("Phase2", "Timeline", "MV3-G12b", "Phase2 is locked by the strict prior-phase Gate barrier; Angular/Go/WS/SQLite real route is not yet verified in this target.").unwrap();
}
