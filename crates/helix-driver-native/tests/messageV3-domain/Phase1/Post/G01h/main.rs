use crate::shared;
use serde_json::json;

/// MV3-G01h：沿 Angular 的 `im_send_message(TASK_CARD)` 入口验证任务卡投影。
#[tokio::test]
async fn mv3_g01h_real_chain() {
    let task_id = shared::resource_name("task-card");
    let title = format!("Phase1 TASK_CARD {task_id}");
    let task_card = json!({
        "id": task_id,
        "title": title,
        "status": "TODO",
        "progress": 0
    });
    shared::assert_isolated_send(
        "Phase1",
        "Post",
        "MV3-G01H",
        "TASK_CARD",
        format!("[任务] {title}"),
        json!({
            "taskCard": task_card.clone(),
            "taskCardBlock": {
                "taskId": task_id,
                "title": title,
                "detail": {
                    "id": task_card["id"],
                    "title": task_card["title"],
                    "status": task_card["status"],
                    "progress": 0
                }
            }
        }),
        vec!["all".to_string()],
        Vec::new(),
        None,
    )
    .await
    .unwrap_or_else(|error| panic!("MV3-G01H FAIL: {error}"));
}
