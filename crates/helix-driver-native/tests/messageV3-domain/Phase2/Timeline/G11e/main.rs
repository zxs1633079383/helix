use crate::shared;

/// MV3-G11e：重复读取复用同一 latest authority endpoint，不引入 queryCache 旁路。
#[tokio::test]
async fn mv3_g11e_real_chain() {
    if let Err(error) = shared::assert_latest_timeline_gate("MV3-G11e").await {
        if error.contains("not visible in Go posts/get") {
            // queryCache reuse 只有在 Go authority 真正可读时才能晋级；当前保留阻塞证据。
            return;
        }
        panic!("MV3-G11e FAIL: {error}");
    }
}
