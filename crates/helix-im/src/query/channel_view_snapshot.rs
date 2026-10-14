//! 当前账号、当前频道的本地 ChannelView 绝对投影。

use helix_core::effect::{Correlation, Effect, GetSpec, SqlValue, StorageOp};

use crate::error::ImError;
use crate::query::DialogListScope;
use crate::state::ChannelId;

pub const QUERY_CHANNEL_VIEW_SNAPSHOT: &str = "im_query_channel_view_snapshot";

/// caller 只拥有频道与 transport correlation；账号和租户必须来自 RuntimeAuth。
pub fn parse_channel_id(payload: &[u8]) -> Result<ChannelId, ImError> {
    let value = serde_json::from_slice::<serde_json::Value>(payload).map_err(|error| {
        ImError::Parse(format!("{QUERY_CHANNEL_VIEW_SNAPSHOT} payload: {error}"))
    })?;
    let object = value.as_object().ok_or_else(|| {
        ImError::Parse(format!(
            "{QUERY_CHANNEL_VIEW_SNAPSHOT} payload must be an object"
        ))
    })?;
    for key in object.keys() {
        if !matches!(key.as_str(), "channelId" | "channel_id" | "req_id") {
            return Err(ImError::Parse(format!(
                "{QUERY_CHANNEL_VIEW_SNAPSHOT} field is not caller-owned: {key}"
            )));
        }
    }
    object
        .get("channelId")
        .or_else(|| object.get("channel_id"))
        .and_then(serde_json::Value::as_str)
        .and_then(ChannelId::from_str)
        .ok_or_else(|| ImError::Parse(format!("{QUERY_CHANNEL_VIEW_SNAPSHOT} requires channelId")))
}

/// 单键读取 canonical channel 行；可见性在读回阶段按 RuntimeAuth 二次校验。
pub fn query_effect(channel_id: ChannelId, corr: Correlation) -> Effect {
    Effect::Persist {
        corr,
        ops: vec![StorageOp::Get(GetSpec {
            table: "channel",
            key_col: "id",
            key_val: SqlValue::Text(channel_id.as_str().to_string()),
        })],
    }
}

/// 返回 `{snapshot}` typed body；缺行、跨租户或非成员均是明确 local miss。
pub fn result_body(
    reply_bytes: &[u8],
    member_bytes: &[u8],
    scope: &DialogListScope,
    channel_id: ChannelId,
) -> serde_json::Value {
    let snapshot = serde_json::from_slice::<Vec<serde_json::Value>>(reply_bytes)
        .ok()
        .and_then(|rows| crate::channel_sync::project_authority_rows(rows, member_bytes, scope))
        .and_then(|items| {
            items.into_iter().find(|row| {
                row.get("id").and_then(serde_json::Value::as_str) == Some(channel_id.as_str())
            })
        });
    serde_json::json!({ "snapshot": snapshot })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_is_runtime_scoped_and_render_ready() {
        let channel_id = ChannelId::from_str("chfixx0000000000000000002a").unwrap();
        let rows = serde_json::to_vec(&serde_json::json!([{
            "id": channel_id.as_str(),
            "team_id": "team-a",
            "members": "[{\"userId\":\"user-a\",\"role\":\"MEMBER\"}]",
            "setting_version": 3,
            "permission_revision": 4
        }]))
        .unwrap();
        let members = serde_json::to_vec(&serde_json::json!([{"channel_id":channel_id.as_str(),"user_id":"user-a","team_id":"team-a","role":"MEMBER"}])).unwrap();
        let body = result_body(
            &rows,
            &members,
            &DialogListScope::new("user-a", "team-a"),
            channel_id,
        );
        assert_eq!(body["snapshot"]["id"], channel_id.as_str());
        assert_eq!(body["snapshot"]["settingVersion"], 3);
    }

    #[test]
    fn snapshot_rejects_cross_tenant_row() {
        let channel_id = ChannelId::from_str("chfixx0000000000000000002a").unwrap();
        let rows = serde_json::to_vec(&serde_json::json!([{
            "id": channel_id.as_str(),
            "team_id": "team-b",
            "user_id": "user-a"
        }]))
        .unwrap();
        let members = serde_json::to_vec(&serde_json::json!([{"channel_id":channel_id.as_str(),"user_id":"user-a","team_id":"team-a","role":"MEMBER"}])).unwrap();
        let body = result_body(
            &rows,
            &members,
            &DialogListScope::new("user-a", "team-a"),
            channel_id,
        );
        assert!(body["snapshot"].is_null());
    }
}
