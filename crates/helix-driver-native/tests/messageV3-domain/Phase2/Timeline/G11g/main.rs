use crate::shared;

/// MV3-G11g：按 server postId 精确读取，断言 Go body、Helix read result 与本地 durable row。
#[tokio::test]
async fn mv3_g11g_real_chain() {
    shared::assert_exact_timeline_gate("MV3-G11g")
        .await
        .unwrap_or_else(|error| panic!("MV3-G11g FAIL: {error}"));
}
