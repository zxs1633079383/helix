use crate::shared;

/// MV3-G02d：保持真实前置缺口可见，不用 fake server 或 HTTP 200 伪造通过。
#[tokio::test]
#[ignore = "NOT_VERIFIED/NOT_APPLICABLE: run explicitly after the named precondition is available"]
async fn mv3_g02d_real_chain() {
    shared::record_unverified("Phase1", "Upload", "MV3-G02d", "G02d is a local TauriHostCommand >64MiB zero-Effect Gate, not a media Go mutation; native host fixture is not yet wired.").unwrap();
}
