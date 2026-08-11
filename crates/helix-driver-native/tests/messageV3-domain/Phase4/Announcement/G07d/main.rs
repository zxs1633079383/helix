use crate::shared;

/// MV3-G07d：真实保存公告后执行已读 mutation，并核对 post updated。
#[tokio::test]
async fn mv3_g07d_real_chain() {
    shared::assert_announcement_read_chain("Phase4", "Announcement", "MV3-G07d")
        .await
        .unwrap_or_else(|error| panic!("MV3-G07d FAIL: {error}"));
}
