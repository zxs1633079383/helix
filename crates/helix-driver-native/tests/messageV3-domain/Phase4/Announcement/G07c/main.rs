use crate::shared;

/// MV3-G07c：真实保存公告后删除，并核对删除公告与版本投影。
#[tokio::test]
async fn mv3_g07c_real_chain() {
    shared::assert_announcement_delete_chain("Phase4", "Announcement", "MV3-G07c")
        .await
        .unwrap_or_else(|error| panic!("MV3-G07c FAIL: {error}"));
}
