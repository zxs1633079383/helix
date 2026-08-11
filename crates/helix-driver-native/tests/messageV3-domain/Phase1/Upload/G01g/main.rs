use crate::shared;
use serde_json::json;

/// MV3-G01g：用 Canvas 生成图已经完成上传的稳定引用验证 TEMPLATE/IMAGE 闭环。
#[tokio::test]
async fn mv3_g01g_real_chain() {
    shared::assert_media_reference_send(
        "Phase1",
        "Upload",
        "MV3-G01g",
        "TEMPLATE",
        shared::resource_name("generated-template"),
        json!({
            "template": {
                "type": "IMAGE",
                "file": shared::completed_media_ref(
                    "picture",
                    "phase1-template",
                    "template-1.png",
                    "image/png",
                    4096,
                    "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
                    json!({"width": 640, "height": 360}),
                )
            }
        }),
        json!({"case": "generated-template-image-stable-reference", "generated_media": true}),
    )
    .await
    .unwrap_or_else(|error| panic!("MV3-G01g FAIL: {error}"));
}
