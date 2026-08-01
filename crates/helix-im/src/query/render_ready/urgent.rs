//! Authoritative expediteMap -> viewer-relative urgent projection.

use serde_json::Value;

pub(crate) struct UrgentProjection<'a> {
    pub requester_id: Option<&'a str>,
    pub target_ids: Vec<&'a str>,
    pub confirmed_ids: Vec<&'a str>,
    pub required_count: usize,
    pub state: &'static str,
    pub can_confirm: bool,
}

pub(crate) fn project<'a>(
    expedite_map: &'a Value,
    viewer_user_id: &str,
    has_server_id: bool,
) -> UrgentProjection<'a> {
    let requester_id = expedite_map
        .get("sender")
        .and_then(Value::as_object)
        .and_then(|sender| sender.get("id"))
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty());
    let recipients = expedite_map.get("recipients").and_then(Value::as_object);
    let required_count = recipients.map(|items| items.len()).unwrap_or_default();
    let mut target_ids = Vec::with_capacity(required_count);
    let mut confirmed_ids = Vec::with_capacity(required_count);
    let mut viewer_pending = false;
    if let Some(recipients) = recipients {
        for (user_id, recipient) in recipients {
            if user_id.is_empty() {
                continue;
            }
            target_ids.push(user_id.as_str());
            let confirmed = recipient
                .get("status")
                .and_then(Value::as_i64)
                .unwrap_or_default()
                > 0;
            if confirmed {
                confirmed_ids.push(user_id.as_str());
            } else if user_id == viewer_user_id {
                viewer_pending = true;
            }
        }
    }
    let required_count = target_ids.len();
    let state = if required_count == 0 {
        "inactive"
    } else if confirmed_ids.len() == required_count {
        "acknowledged"
    } else {
        "active"
    };
    let can_confirm = has_server_id
        && !viewer_user_id.is_empty()
        && requester_id != Some(viewer_user_id)
        && viewer_pending;

    UrgentProjection {
        requester_id,
        target_ids,
        confirmed_ids,
        required_count,
        state,
        can_confirm,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn derives_sender_recipient_and_confirmed_views_in_one_pass() {
        let value = json!({
            "sender":{"id":"u1"},
            "recipients":{"u2":{"status":0},"u3":{"status":1}}
        });
        let projection = project(&value, "u2", true);
        assert_eq!(projection.requester_id, Some("u1"));
        assert_eq!(projection.target_ids.len(), 2);
        assert_eq!(projection.confirmed_ids, ["u3"]);
        assert_eq!(projection.required_count, 2);
        assert_eq!(projection.state, "active");
        assert!(projection.can_confirm);
    }

    /// INV-12：发起人即使被写进 `recipients`，也不得派生出待确认动作。
    ///
    /// 独立守护 `can_confirm` 里的 `requester_id != Some(viewer_user_id)` 分支——
    /// 去掉该条件本用例必须变红（该分支此前无任何测试覆盖）。
    #[test]
    fn requester_listed_as_recipient_never_becomes_confirmable() {
        let value = json!({
            "sender":{"id":"u1"},
            "recipients":{"u1":{"status":0},"u2":{"status":0}}
        });
        let requester = project(&value, "u1", true);
        assert_eq!(requester.requester_id, Some("u1"));
        // 绝对面如实保留发起人在 target 集合内，只有 viewer-local 的动作面被关掉。
        assert!(requester.target_ids.contains(&"u1"));
        assert!(
            !requester.can_confirm,
            "加急发起人不得对自己发起的加急出现待确认动作（INV-12）"
        );
        // 同一份绝对 map 下普通被点名成员不受影响。
        assert!(project(&value, "u2", true).can_confirm);
    }

    #[test]
    fn fully_confirmed_is_acknowledged_and_not_confirmable() {
        let value = json!({
            "sender":{"id":"u1"},
            "recipients":{"u2":{"status":1}}
        });
        let projection = project(&value, "u2", true);
        assert_eq!(projection.state, "acknowledged");
        assert!(!projection.can_confirm);
    }
}
