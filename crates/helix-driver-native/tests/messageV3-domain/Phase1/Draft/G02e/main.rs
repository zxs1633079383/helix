use crate::shared;

/// MV3-G02e：保持真实前置缺口可见，不用 fake server 或 HTTP 200 伪造通过。
#[tokio::test]
#[ignore = "NOT_VERIFIED/NOT_APPLICABLE: run explicitly after the named precondition is available"]
async fn mv3_g02e_real_chain() {
    shared::record_unverified("Phase1", "Draft", "MV3-G02e", "draft is local HostStorage by contract; no Go 8066 mutation exists, so this is not a fake HTTP test.").unwrap();
}
