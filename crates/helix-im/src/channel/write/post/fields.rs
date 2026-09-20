//! Post visibility and urgency field shaping.

pub(super) fn visible_to_user(msg_type: &str, viewers: &[String], auth_user_id: &str) -> bool {
    if msg_type == "NOTICE" || auth_user_id.is_empty() || viewers.is_empty() {
        return true;
    }
    viewers
        .iter()
        .any(|viewer| viewer == "all" || viewer == auth_user_id)
}

pub(super) fn sender_user_id_from_value(data: &serde_json::Value) -> String {
    data.get("userSnapshot")
        .and_then(|s| s.get("userId"))
        .and_then(|v| v.as_str())
        .or_else(|| data.get("userId").and_then(|v| v.as_str()))
        .unwrap_or("")
        .to_string()
}

pub(super) fn string_array(value: Option<&serde_json::Value>) -> Vec<String> {
    match value {
        Some(serde_json::Value::Array(items)) => items
            .iter()
            .filter_map(|item| item.as_str().map(str::to_string))
            .collect(),
        Some(serde_json::Value::String(s)) if !s.is_empty() => vec![s.clone()],
        _ => Vec::new(),
    }
}

pub(super) fn urgent_from_fields(
    fields: &crate::sync_session::PostFields,
    auth_user_id: &str,
) -> (bool, Vec<String>) {
    let mut candidates = Vec::new();
    if let Ok(expedite) = serde_json::from_str::<serde_json::Value>(&fields.expedite_map) {
        collect_urgent_targets(&expedite, &mut candidates);
    }
    if let Ok(props) = serde_json::from_str::<serde_json::Value>(&fields.props) {
        collect_urgent_from_props(&props, &mut candidates);
    }
    urgent_hit_from_candidates(candidates, auth_user_id)
}

pub(super) fn urgent_from_value(
    data: &serde_json::Value,
    auth_user_id: &str,
) -> (bool, Vec<String>) {
    let mut candidates = Vec::new();
    if let Some(expedite) = data.get("expediteMap").or_else(|| data.get("expedite_map")) {
        collect_urgent_targets(expedite, &mut candidates);
    }
    if let Some(props) = data.get("props") {
        collect_urgent_from_props(props, &mut candidates);
    }
    urgent_hit_from_candidates(candidates, auth_user_id)
}

fn collect_urgent_from_props(props: &serde_json::Value, out: &mut Vec<String>) {
    for key in ["expedite", "urgent", "urgentUsers", "urgentUserIds"] {
        if let Some(value) = props.get(key) {
            collect_urgent_targets(value, out);
        }
    }
}

/// 承载「加急目标集合」的容器键：命中即只从这些键下钻，兄弟键一律不是目标。
const URGENT_TARGET_CONTAINER_KEYS: [&str; 4] = ["recipients", "users", "userIds", "targets"];

/// 单条目形态 `{"id": "u1"}` / `{"userId": "u1"}`：目标在 value 上，键本身不是 userId。
const URGENT_TARGET_ID_KEYS: [&str; 3] = ["id", "userId", "user_id"];

/// expediteMap / recipient 条目里描述「发起人与元数据」的结构键。
///
/// 它们既不是 userId，也不得被下钻——`sender` 一旦被当成目标容器，
/// 发起人就会出现在自己消息的加急名单里（违反 INV-12：禁止 sender/receiver 状态互抄）。
const URGENT_STRUCTURE_KEYS: [&str; 11] = [
    "sender",
    "status",
    "confirmAt",
    "confirm_at",
    "name",
    "teamId",
    "team_id",
    "createAt",
    "create_at",
    "updateAt",
    "update_at",
];

fn collect_urgent_targets(value: &serde_json::Value, out: &mut Vec<String>) {
    match value {
        serde_json::Value::String(s) if !s.is_empty() => out.push(s.clone()),
        serde_json::Value::Bool(true) => out.push("all".to_string()),
        serde_json::Value::Array(items) => {
            for item in items {
                collect_urgent_targets(item, out);
            }
        }
        serde_json::Value::Object(map) => collect_urgent_targets_from_object(map, out),
        _ => {}
    }
}

/// 从一个 object 结点收集加急目标。
///
/// 三种真实形态，按优先级判定，**互斥**（判定出结构化形态后绝不再退回扁平枚举）：
/// 1. 容器形态 `{"sender":…, "recipients":{userId:…}}` / `{"users":[…]}`：只下钻容器键，
///    `sender` 及其它兄弟键一律跳过（发起人不是加急目标，INV-12）。
/// 2. 单条目形态 `{"id":"u1"}` / `{"userId":"u1"}`：目标在 value 上。
/// 3. 扁平映射 `{userId: true}` / `{userId: {"status":0}}`：键即 userId，只取键不再下钻——
///    下钻会把 `status` / `confirmAt` / 显示名等结构字段误当成 userId 推入目标集合。
fn collect_urgent_targets_from_object(
    map: &serde_json::Map<String, serde_json::Value>,
    out: &mut Vec<String>,
) {
    let mut structured = map.contains_key("sender");
    for key in URGENT_TARGET_CONTAINER_KEYS {
        if let Some(value) = map.get(key) {
            structured = true;
            collect_urgent_targets(value, out);
        }
    }
    if structured {
        return;
    }
    if let Some(value) = URGENT_TARGET_ID_KEYS.iter().find_map(|key| map.get(*key)) {
        collect_urgent_targets(value, out);
        return;
    }
    for (key, value) in map {
        if URGENT_STRUCTURE_KEYS.contains(&key.as_str()) {
            continue;
        }
        if matches!(value, serde_json::Value::Bool(false)) {
            continue;
        }
        out.push(key.clone());
    }
}

fn urgent_hit_from_candidates(
    mut candidates: Vec<String>,
    auth_user_id: &str,
) -> (bool, Vec<String>) {
    candidates.retain(|id| !id.is_empty());
    candidates.sort();
    candidates.dedup();
    let hit = if auth_user_id.is_empty() {
        !candidates.is_empty()
    } else {
        candidates
            .iter()
            .any(|id| id == "all" || id == auth_user_id)
    };
    (hit, candidates)
}

/// Preserves the remote post clocks and preview fields in the recovered dialog snapshot.
pub(super) fn sync_last_post_json(fields: &crate::sync_session::PostFields) -> String {
    let mut data = serde_json::json!({
        "id": fields.id.clone(),
        "temporaryId": fields.temporary_id.clone(),
        "channelId": fields.channel_id.clone(),
        // 仅作为摘要公司作用域输入；移除后保持既有 sync lastPost 字段集。
        "teamId": fields.team_id.clone(),
        "userId": fields.user_id.clone(),
        "type": fields.msg_type.clone(),
        "message": fields.message.clone(),
        "simpleMessage": fields.simple_message.clone(),
        "props": serde_json::from_str::<serde_json::Value>(&fields.props).unwrap_or(serde_json::Value::Null),
        "viewers": fields.viewers.clone(),
        "mentions": fields.mentions.clone(),
        "expediteMap": serde_json::from_str::<serde_json::Value>(&fields.expedite_map).unwrap_or(serde_json::Value::Null),
        "createAt": fields.create_at,
        "updateAt": if fields.update_at > 0 { fields.update_at } else { fields.create_at },
    });
    crate::message_summary::attach_parts(&mut data);
    if let Some(object) = data.as_object_mut() {
        object.remove("teamId");
    }
    data.to_string()
}

#[cfg(test)]
mod tests {
    use super::sync_last_post_json;
    use crate::sync_session::PostFields;
    use serde_json::json;

    #[test]
    fn sync_last_post_contains_member_notice_parts() {
        let fields = PostFields {
            team_id: "company-a".to_string(),
            msg_type: "NOTICE".to_string(),
            message: "member notice".to_string(),
            props: json!({
                "type": "join",
                "operator": {"id": "operator"},
                "users": [{"id": "target", "name": "旧目标名"}]
            })
            .to_string(),
            ..PostFields::default()
        };
        let data: serde_json::Value =
            serde_json::from_str(&sync_last_post_json(&fields)).expect("sync last post JSON");
        assert_eq!(
            data["summaryParts"],
            json!([
                {"kind":"user","userId":"operator","companyId":"company-a"},
                {"kind":"text","text":"邀请"},
                {"kind":"user","userId":"target","companyId":"company-a"},
                {"kind":"text","text":"加入群聊"}
            ])
        );
        assert!(!data["summaryParts"].to_string().contains("旧"));
    }
}
