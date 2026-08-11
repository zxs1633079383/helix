use crate::shared;
use serde_json::json;

/// MV3-G02a：独立 Gate 真实链测试；要求业务 SUCCESS、WS/Projection 与最终状态证据。
#[tokio::test]
async fn mv3_g02a_real_chain() {
    shared::assert_isolated_send(
        "Phase1",
        "Post",
        "MV3-G02a",
        "TEXT",
        shared::resource_name("text-persist"),
        json!({}),
        vec!["all".to_string()],
        Vec::new(),
        None,
    )
    .await
    .unwrap_or_else(|error| panic!("MV3-G02a FAIL: {error}"));
}
