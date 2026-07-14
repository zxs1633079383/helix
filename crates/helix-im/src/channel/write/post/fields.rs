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

fn collect_urgent_targets(value: &serde_json::Value, out: &mut Vec<String>) {
    match value {
        serde_json::Value::String(s) if !s.is_empty() => out.push(s.clone()),
        serde_json::Value::Bool(true) => out.push("all".to_string()),
        serde_json::Value::Array(items) => {
            for item in items {
                collect_urgent_targets(item, out);
            }
        }
        serde_json::Value::Object(map) => {
            for (key, value) in map {
                if key == "users" || key == "userIds" || key == "targets" {
                    collect_urgent_targets(value, out);
                } else if value.as_bool().unwrap_or(true) {
                    out.push(key.clone());
                    collect_urgent_targets(value, out);
                }
            }
        }
        _ => {}
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

pub(super) fn sync_last_post_json(fields: &crate::sync_session::PostFields) -> String {
    serde_json::json!({
        "id": fields.id.clone(),
        "temporaryId": fields.temporary_id.clone(),
        "channelId": fields.channel_id.clone(),
        "userId": fields.user_id.clone(),
        "type": fields.msg_type.clone(),
        "message": fields.message.clone(),
        "simpleMessage": fields.simple_message.clone(),
        "props": serde_json::from_str::<serde_json::Value>(&fields.props).unwrap_or(serde_json::Value::Null),
        "viewers": fields.viewers.clone(),
        "mentions": fields.mentions.clone(),
        "expediteMap": serde_json::from_str::<serde_json::Value>(&fields.expedite_map).unwrap_or(serde_json::Value::Null),
        "createAt": fields.create_at,
    })
    .to_string()
}
