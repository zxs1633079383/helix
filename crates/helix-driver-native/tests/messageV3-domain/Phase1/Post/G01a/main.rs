use crate::shared;
use serde_json::json;

/// MV3-G01a：独立 Gate 真实链测试；要求业务 SUCCESS、WS/Projection 与最终状态证据。
#[tokio::test]
async fn mv3_g01a_real_chain() {
    shared::assert_isolated_send(
        "Phase1",
        "Post",
        "MV3-G01a",
        "TEXT",
        shared::resource_name("text"),
        json!({}),
        vec!["all".to_string()],
        Vec::new(),
        None,
    )
    .await
    .unwrap_or_else(|error| panic!("MV3-G01a FAIL: {error}"));
}
