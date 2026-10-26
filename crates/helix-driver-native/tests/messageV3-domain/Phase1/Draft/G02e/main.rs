use crate::shared;

/// MV3-G02e：通过公开 save/query Tick 回放验证本地 HostStorage 草稿闭环。
#[tokio::test]
async fn mv3_g02e_real_chain() {
    shared::assert_local_draft_chain("Phase1", "Draft", "MV3-G02e")
        .await
        .unwrap_or_else(|error| panic!("MV3-G02e real chain failed: {error}"));
}
