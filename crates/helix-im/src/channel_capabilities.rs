//! 当前 viewer 的频道治理能力：角色事实由 WS 成员投影记忆，能力矩阵由 Go 回声提供。

use crate::state::{ChannelId, ImState};
use serde_json::{json, Value};

pub(crate) fn remember_viewer_role(
    state: &mut ImState,
    channel_id: ChannelId,
    data: &Value,
    viewer_user_id: &str,
) {
    if viewer_user_id.is_empty() {
        state.viewer_channel_roles.remove(&channel_id);
        return;
    }
    if crate::channel_write::collect_member_leaves(data)
        .iter()
        .any(|id| id == viewer_user_id)
    {
        state.viewer_channel_roles.remove(&channel_id);
        return;
    }
    let direct = data.get("role").and_then(Value::as_str);
    let from_members = crate::channel_write::collect_members(data)
        .into_iter()
        .find(|member| member.user_id == viewer_user_id)
        .map(|member| member.role);
    let owner = data
        .get("owner")
        .and_then(|owner| owner.get("id").or_else(|| owner.get("userId")))
        .and_then(Value::as_str)
        .filter(|id| *id == viewer_user_id)
        .map(|_| "CREATOR".to_string());
    if let Some(role) = owner
        .or(from_members.map(|role| matrix_role_key(&role).to_string()))
        .or(direct.map(|role| matrix_role_key(role).to_string()))
    {
        state.viewer_channel_roles.insert(channel_id, role);
    }
}

pub(crate) fn remember_role_update(
    state: &mut ImState,
    channel_id: ChannelId,
    data: &Value,
    viewer_user_id: &str,
) {
    let targets_viewer = data
        .get("userIds")
        .or_else(|| data.get("user_ids"))
        .and_then(Value::as_array)
        .is_some_and(|ids| ids.iter().any(|id| id.as_str() == Some(viewer_user_id)));
    if targets_viewer {
        if let Some(role) = data.get("role").and_then(Value::as_str) {
            state
                .viewer_channel_roles
                .insert(channel_id, matrix_role_key(role).to_string());
        }
    }
}

pub(crate) fn for_viewer(matrix: Option<&Value>, role: Option<&str>) -> Value {
    let selected = role
        .and_then(|role| matrix.and_then(|value| value.get(role)))
        .and_then(Value::as_object);
    let flag = |key: &str| {
        selected
            .and_then(|capabilities| capabilities.get(key))
            .and_then(Value::as_bool)
            .unwrap_or(false)
    };
    json!({
        "canCloseChannel": flag("canCloseChannel"),
        "canLeaveChannel": flag("canLeaveChannel"),
        "canManageSettings": flag("canManageSettings"),
        "canManageMembers": flag("canManageMembers"),
    })
}

fn matrix_role_key(role: &str) -> &'static str {
    match role {
        "OWNER" | "CREATOR" => "CREATOR",
        "ADMIN" | "MANAGER" | "MANGER" => "MANAGER",
        "BOSS" => "BOSS",
        _ => "MEMBER",
    }
}
