//! 消息动作能力策略：只根据 Helix 已持有的服务端锚、viewer 身份和交互终态生成布尔能力。

use serde_json::{json, Value};

pub(crate) fn build(
    has_server_id: bool,
    is_self: bool,
    viewer_user_id: &str,
    props: &Value,
    expedite_map: &Value,
    can_retry: bool,
    revoked: bool,
) -> Value {
    let can_mark_read = has_server_id && !is_self;
    let can_view_receipt = has_server_id && is_self;
    let can_urgent_confirm = has_server_id
        && !is_self
        && !viewer_user_id.is_empty()
        && expedite_map
            .get("recipients")
            .and_then(Value::as_object)
            .and_then(|recipients| recipients.get(viewer_user_id))
            .is_some_and(|recipient| {
                recipient
                    .get("status")
                    .and_then(Value::as_i64)
                    .unwrap_or_default()
                    <= 0
            });
    let template = props.get("template").and_then(Value::as_object);
    let already_confirmed = template
        .and_then(|value| value.get("userIds").or_else(|| value.get("user_ids")))
        .and_then(Value::as_array)
        .is_some_and(|ids| ids.iter().any(|id| id.as_str() == Some(viewer_user_id)));
    let can_template_confirm = has_server_id
        && !is_self
        && !viewer_user_id.is_empty()
        && template.is_some()
        && !already_confirmed;
    let can_locate = has_server_id && !revoked;

    json!({
        "canReply": has_server_id,
        "canForward": has_server_id,
        "canReact": has_server_id,
        "canUrgent": has_server_id,
        "canCreateTopic": has_server_id,
        "canRevoke": has_server_id && is_self,
        "canRetry": can_retry,
        "canUrgentConfirm": can_urgent_confirm,
        "canMarkRead": can_mark_read,
        "canViewReceipt": can_view_receipt,
        "canTemplateConfirm": can_template_confirm,
        "canLocate": can_locate,
    })
}
