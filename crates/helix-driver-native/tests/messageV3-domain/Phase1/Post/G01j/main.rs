use crate::shared;
use serde_json::json;

/// MV3-G01J：通过 Helix `im_average_publish` 调 Java，再以 Go/WS 评分卡回声验收。
#[tokio::test]
async fn mv3_g01j_real_chain() {
    let title = shared::resource_name("average-score");
    shared::assert_java_engagement_create_chain(
        "Phase1",
        "Post",
        "MV3-G01J",
        "im_average_publish",
        "/average/publish",
        json!({
            "title": title.clone(),
            "content": "Phase1 real-chain average score",
            "max_score": "10",
            "min_score": "0",
            "is_del_max_min": false,
            "is_anonymous": false,
            "cutoff": 4_000_000_000_000_i64,
            "has_decimal": true,
            "decimal_places": 1
        }),
        "AVERAGE_SCORE",
        &title,
        "averageScore",
    )
    .await
    .unwrap_or_else(|error| panic!("MV3-G01J FAIL: {error}"));
}
