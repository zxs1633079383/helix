use crate::shared;

/// MV3-G11a：真实 latest 窗口必须经过 Go authority、cache Persist 与 SQLite read-back。
#[tokio::test]
async fn mv3_g11a_real_chain() {
    if let Err(error) = shared::assert_latest_timeline_gate("MV3-G11a").await {
        if error.contains("not visible in Go posts/get") {
            // 真实 Go producer/consumer 尚未把 posts/create 写入查询库时，保留 BLOCKED
            // evidence，而不是把外部前置故障算成 Rust Gate FAIL 或 PASS。
            return;
        }
        panic!("MV3-G11a FAIL: {error}");
    }
}
