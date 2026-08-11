use crate::shared;
use serde_json::json;

/// MV3-G01d：用 Angular 文件服务已完成的稳定附件引用验证真实 Go 消息闭环。
#[tokio::test]
async fn mv3_g01d_real_chain() {
    shared::assert_media_reference_send(
        "Phase1",
        "Upload",
        "MV3-G01d",
        "FILE",
        "phase1-file.txt".to_string(),
        json!({
            "file": shared::completed_media_ref(
                "attachment",
                "phase1-file",
                "phase1-file.txt",
                "text/plain",
                128,
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                json!({}),
            )
        }),
        json!({"case": "completed-file-reference"}),
    )
    .await
    .unwrap_or_else(|error| panic!("MV3-G01d FAIL: {error}"));
}
