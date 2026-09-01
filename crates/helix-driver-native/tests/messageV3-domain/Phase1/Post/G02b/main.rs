use crate::shared;

/// MV3-G02b：独立 Gate 真实链测试；要求业务 SUCCESS、WS/Projection 与最终状态证据。
#[tokio::test]
async fn mv3_g02b_real_chain() {
    let (_guard, context) = shared::remote_setup().await;
    let invalid_channel = format!(
        "chfixx{:020x}",
        shared::RUN_SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    );
    shared::run_helix_send_failure(&context, &invalid_channel, "Phase1", "Post", "MV3-G02b")
        .await
        .unwrap_or_else(|error| panic!("MV3-G02b FAIL: {error}"));
}
