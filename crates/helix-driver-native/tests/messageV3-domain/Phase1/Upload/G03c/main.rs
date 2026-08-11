use crate::shared;

/// MV3-G03c：保持真实前置缺口可见，不用 fake server 或 HTTP 200 伪造通过。
#[tokio::test]
#[ignore = "NOT_VERIFIED/NOT_APPLICABLE: run explicitly after the named precondition is available"]
async fn mv3_g03c_real_chain() {
    shared::record_unverified(
        "Phase1",
        "Upload",
        "MV3-G03c",
        "Go media fixture/forward path is not verified after the live media.type NOT NULL blocker.",
    )
    .unwrap();
}
