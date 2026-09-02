//! 频道置顶集合的账号隔离本地绝对投影。

use helix_core::effect::{
    BatchDeleteSpec, Correlation, Effect, GetSpec, SqlValue, StorageOp, UpsertSpec,
};

use crate::error::ImError;
use crate::state::ChannelId;

pub const QUERY_PINNED_PROJECTION: &str = "im_query_pinned_projection";

/// 只接受 caller 拥有的频道与 transport correlation 字段。
pub fn parse_channel_id(payload: &[u8]) -> Result<ChannelId, ImError> {
    let value = serde_json::from_slice::<serde_json::Value>(payload)
        .map_err(|error| ImError::Parse(format!("{QUERY_PINNED_PROJECTION} payload: {error}")))?;
    let object = value.as_object().ok_or_else(|| {
        ImError::Parse(format!(
            "{QUERY_PINNED_PROJECTION} payload must be an object"
        ))
    })?;
    for key in object.keys() {
        if !matches!(key.as_str(), "channelId" | "channel_id" | "req_id") {
            return Err(ImError::Parse(format!(
                "{QUERY_PINNED_PROJECTION} field is not caller-owned: {key}"
            )));
        }
    }
    object
        .get("channelId")
        .or_else(|| object.get("channel_id"))
        .and_then(serde_json::Value::as_str)
        .and_then(ChannelId::from_str)
        .ok_or_else(|| ImError::Parse(format!("{QUERY_PINNED_PROJECTION} requires channelId")))
}

/// 账号与频道共同组成本地 projection key，禁止跨账号复用同名频道缓存。
pub fn projection_key(account_id: &str, channel_id: ChannelId) -> Result<String, ImError> {
    if account_id.is_empty() {
        return Err(ImError::Parse(
            "pinned projection requires RuntimeAuth account".to_string(),
        ));
    }
    Ok(format!("{account_id}:{}", channel_id.as_str()))
}

/// 构造单行本地置顶投影读取，不触发 HTTP。
pub fn query_effect(projection_key: String, corr: Correlation) -> Effect {
    Effect::Persist {
        corr,
        ops: vec![StorageOp::Get(GetSpec {
            table: "channel_pinned_projection",
            key_col: "projection_key",
            key_val: SqlValue::Text(projection_key),
        })],
    }
}

/// 把一次权威 HTTP 列表保存为账号+频道单行绝对投影。
pub fn persist_effect(
    projection_key: String,
    account_id: String,
    channel_id: ChannelId,
    raw_body: &[u8],
    corr: Correlation,
) -> Result<Effect, ImError> {
    let body = serde_json::from_slice::<serde_json::Value>(raw_body)
        .map_err(|error| ImError::Parse(format!("pinned projection body: {error}")))?;
    Ok(Effect::Persist {
        corr,
        ops: vec![StorageOp::BatchUpsert(UpsertSpec::new(
            "channel_pinned_projection",
            vec![vec![
                ("projection_key".to_string(), SqlValue::Text(projection_key)),
                ("account_id".to_string(), SqlValue::Text(account_id)),
                (
                    "channel_id".to_string(),
                    SqlValue::Text(channel_id.as_str().to_string()),
                ),
                ("body".to_string(), SqlValue::Text(body.to_string())),
            ]],
            Some("projection_key"),
        ))],
    })
}

/// pin/unpin authority 到达后删除旧绝对集合；当前事件仍独立更新消息投影。
pub fn invalidate_effect(account_id: &str, projection_key: String) -> Effect {
    Effect::PersistFire {
        ops: vec![StorageOp::BatchDelete(BatchDeleteSpec {
            table: "channel_pinned_projection",
            scope_col: "account_id",
            scope_val: SqlValue::Text(account_id.to_string()),
            key_col: "projection_key",
            key_vals: vec![SqlValue::Text(projection_key)],
        })],
    }
}

/// 把本地 Get 回包投影成 `{cached, body}`，缺行不是错误而是远端 fallback 信号。
pub fn result_body(reply: &bytes::Bytes) -> Result<serde_json::Value, ImError> {
    let rows = helix_core::port_codec::rows_from_reply_bytes(reply)
        .map_err(|error| ImError::Parse(format!("pinned projection readback: {error}")))?;
    let Some(body) = rows
        .first()
        .and_then(|row| row.iter().find(|(column, _)| column == "body"))
        .and_then(|(_, value)| match value {
            SqlValue::Text(value) => Some(value.as_str()),
            _ => None,
        })
    else {
        return Ok(serde_json::json!({ "cached": false }));
    };
    let body = serde_json::from_str::<serde_json::Value>(body)
        .map_err(|error| ImError::Parse(format!("pinned projection cached body: {error}")))?;
    Ok(serde_json::json!({ "cached": true, "body": body }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use helix_core::effect::Row;

    /// 缺行是明确 cache miss，不伪装为空置顶集合。
    #[test]
    fn missing_row_returns_cache_miss() {
        let reply = helix_core::port_codec::rows_to_reply_bytes(&[]);
        assert_eq!(
            result_body(&reply).unwrap(),
            serde_json::json!({ "cached": false })
        );
    }

    /// 已持久化的后端绝对响应体必须原样回放给 Host normalizer。
    #[test]
    fn cached_row_returns_authority_body() {
        let row: Row = vec![(
            "body".to_string(),
            SqlValue::Text(
                serde_json::json!({ "status": "SUCCESS", "data": [{ "post": { "id": "p1" } }] })
                    .to_string(),
            ),
        )];
        let reply = helix_core::port_codec::rows_to_reply_bytes(&[row]);
        let result = result_body(&reply).unwrap();
        assert_eq!(result["cached"], true);
        assert_eq!(result["body"]["data"][0]["post"]["id"], "p1");
    }

    /// 每次 pin/unpin 都推进失效代次并删除账号级单行投影。
    #[test]
    fn invalidation_advances_epoch_and_deletes_projection() {
        let channel_id = ChannelId::from_str("chfixx0000000000000000002a").unwrap();
        let mut state = crate::state::ImState::new();
        let first = state
            .invalidate_pinned_projection("user-a", channel_id)
            .expect("valid account creates invalidation");
        let second = state
            .invalidate_pinned_projection("user-a", channel_id)
            .expect("second invalidation remains valid");
        assert_eq!(state.pinned_projection_epochs.get(&channel_id), Some(&2));
        for effect in [first, second] {
            assert!(matches!(
                effect,
                Effect::PersistFire { ops }
                    if matches!(ops.first(), Some(StorageOp::BatchDelete(spec))
                        if spec.table == "channel_pinned_projection"
                            && spec.scope_col == "account_id")
            ));
        }
    }
}
