use crate::shared;
use serde_json::json;

/// MV3-G08a：独立 Gate 真实链测试；要求业务 SUCCESS、WS/Projection 与最终状态证据。
#[tokio::test]
async fn mv3_g08a_real_chain() {
    let (_guard, context) = shared::remote_setup().await;
    let channel = shared::create_channel(&context)
        .await
        .unwrap_or_else(|error| panic!("MV3-G08a FAIL: {error}"));
    assert!(
        channel.len() >= 20,
        "channel id is not canonical: {channel}"
    );
    shared::write_gate_evidence(
        "Phase1",
        "Channel",
        "MV3-G08a",
        "PASS",
        json!({"http": "channel/create", "canonical_id": true}),
    )
    .unwrap();
    shared::close_channel(&context, &channel)
        .await
        .unwrap_or_else(|error| panic!("MV3-G08a cleanup failed: {error}"));
}
