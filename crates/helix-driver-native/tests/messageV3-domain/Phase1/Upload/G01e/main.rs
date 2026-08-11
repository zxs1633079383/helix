use crate::shared;
use serde_json::json;

/// MV3-G01e：分别验证 AUDIO 与 VIDEO 的 Host 元数据在真实 Go 消息闭环中保持稳定。
#[tokio::test]
async fn mv3_g01e_real_chain() {
    shared::assert_media_reference_send(
        "Phase1",
        "Upload",
        "MV3-G01e",
        "AUDIO",
        "phase1-audio.m4a".to_string(),
        json!({
            "file": shared::completed_media_ref(
                "attachment",
                "phase1-audio",
                "phase1-audio.m4a",
                "audio/mp4",
                4096,
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                json!({"duration": 2.0}),
            )
        }),
        json!({"case": "audio", "required_metadata": {"duration": 2, "dimensions": "absent"}}),
    )
    .await
    .unwrap_or_else(|error| panic!("MV3-G01e AUDIO FAIL: {error}"));
    shared::assert_media_reference_send(
        "Phase1",
        "Upload",
        "MV3-G01e",
        "VIDEO",
        "phase1-video.mp4".to_string(),
        json!({
            "file": shared::completed_media_ref(
                "attachment",
                "phase1-video",
                "phase1-video.mp4",
                "video/mp4",
                8192,
                "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                json!({"duration": 2.0, "width": 320, "height": 180}),
            )
        }),
        json!({"case": "video", "required_metadata": {"duration": 2, "dimensions": "320x180"}}),
    )
    .await
    .unwrap_or_else(|error| panic!("MV3-G01e VIDEO FAIL: {error}"));
}
