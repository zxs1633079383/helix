use crate::shared;

/// MV3-G07a：真实保存公告后读取公告列表，并核对 read result。
#[tokio::test]
async fn mv3_g07a_real_chain() {
    shared::assert_announcement_list_chain("Phase4", "Announcement", "MV3-G07a")
        .await
        .unwrap_or_else(|error| panic!("MV3-G07a FAIL: {error}"));
}
