use crate::shared;

/// MV3-G03c：执行真实前置证据写入，保留媒体 fixture/forward 缺口。
#[tokio::test]
async fn mv3_g03c_real_chain() {
    shared::record_unverified(
        "Phase1",
        "Upload",
        "MV3-G03c",
        "Go media fixture/forward path is not verified after the live media.type NOT NULL blocker.",
    )
    .unwrap();
}
