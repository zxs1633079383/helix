use crate::shared;

/// MV3-G03c：按 Angular 图片 preview 的 postIds 转发链验证稳定媒体引用不重复上传。
#[tokio::test]
async fn mv3_g03c_real_chain() {
    shared::assert_image_forward_chain("Phase1", "Upload", "MV3-G03c")
        .await
        .unwrap_or_else(|error| panic!("MV3-G03c FAIL: {error}"));
}
