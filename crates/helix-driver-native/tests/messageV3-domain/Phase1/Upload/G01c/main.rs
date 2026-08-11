use crate::shared;

/// MV3-G01c：保持真实前置缺口可见，不用 fake server 或 HTTP 200 伪造通过。
#[tokio::test]
#[ignore = "NOT_VERIFIED/NOT_APPLICABLE: run explicitly after the named precondition is available"]
async fn mv3_g01c_real_chain() {
    shared::record_unverified("Phase1", "Upload", "MV3-G01c", "Go media prepare currently violates live media.type NOT NULL; source: internal/store/sqlstore/media_repository.go Create omits SetType.").unwrap();
}
