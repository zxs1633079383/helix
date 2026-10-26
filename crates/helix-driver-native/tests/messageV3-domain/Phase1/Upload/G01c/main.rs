use crate::shared;
use serde_json::json;

/// MV3-G01c：用两条 Angular/Java/OSS 已完成的稳定图片引用验证真实 Go 消息闭环。
#[tokio::test]
async fn mv3_g01c_real_chain() {
    shared::assert_media_reference_send(
        "Phase1",
        "Upload",
        "MV3-G01c",
        "IMAGE",
        shared::resource_name("multiple-images"),
        json!({
            "imagePosition": "bottom",
            "files": [
                shared::completed_media_ref(
                    "picture",
                    "phase1-image-a",
                    "phase1-image-a.png",
                    "image/png",
                    1024,
                    "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                    json!({"width": 320, "height": 180}),
                ),
                shared::completed_media_ref(
                    "picture",
                    "phase1-image-b",
                    "phase1-image-b.jpg",
                    "image/jpeg",
                    2048,
                    "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                    json!({"width": 640, "height": 360}),
                )
            ]
        }),
        json!({"case": "two-image-stable-reference"}),
    )
    .await
    .unwrap_or_else(|error| panic!("MV3-G01c FAIL: {error}"));
}
