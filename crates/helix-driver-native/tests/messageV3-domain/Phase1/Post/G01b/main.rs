use crate::shared;
use serde_json::json;

/// MV3-G01b：独立 Gate 真实链测试；要求业务 SUCCESS、WS/Projection 与最终状态证据。
#[tokio::test]
async fn mv3_g01b_real_chain() {
    shared::assert_isolated_send(
        "Phase1",
        "Post",
        "MV3-G01b",
        "TEMPLATE",
        shared::resource_name("template"),
        json!({}),
        vec!["all".to_string()],
        vec!["all".to_string()],
        None,
    )
    .await
    .unwrap_or_else(|error| panic!("MV3-G01b FAIL: {error}"));
}
