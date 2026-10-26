use crate::shared;

/// MV3-G02C：通过一次性 Go 故障注入验证 failed row → retry → sent 全链路。
#[tokio::test]
async fn mv3_g02c_real_chain() {
    shared::assert_retry_send_chain("Phase1", "Post", "MV3-G02C")
        .await
        .unwrap_or_else(|error| panic!("MV3-G02C FAIL: {error}"));
}
