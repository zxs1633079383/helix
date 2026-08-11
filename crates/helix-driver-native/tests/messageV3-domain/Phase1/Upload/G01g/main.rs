use crate::shared;

/// MV3-G01g：执行真实前置证据写入，保留 Go media.type 约束错误。
#[tokio::test]
async fn mv3_g01g_real_chain() {
    shared::record_unverified("Phase1", "Upload", "MV3-G01g", "Go media prepare currently violates live media.type NOT NULL; source: internal/store/sqlstore/media_repository.go Create omits SetType.").unwrap();
}
