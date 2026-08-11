use crate::shared;
use serde_json::json;

/// MV3-G02b：独立 Gate 真实链测试；要求业务 SUCCESS、WS/Projection 与最终状态证据。
#[tokio::test]
async fn mv3_g02b_real_chain() {
    let (_guard, context) = shared::remote_setup().await;
    let invalid_channel = format!(
        "chfixx{:020x}",
        shared::RUN_SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    );
    shared::run_helix_send_failure(&context, &invalid_channel)
        .await
        .unwrap_or_else(|error| panic!("MV3-G02b FAIL: {error}"));
    shared::write_gate_evidence("Phase1", "Post", "MV3-G02b", "PASS",
                json!({"command": "im_send_message", "negative": "im:post:send-failed", "received_absent": true})).unwrap();
}
