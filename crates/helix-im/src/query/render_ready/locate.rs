//! G-11 消息定位的 render-ready 投影。

use serde_json::Value;
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum LocateProjectionError {
    #[error("locate response omitted target message {0}")]
    TargetMissing(String),
    #[error(
        "locate response message {message_id} belongs to channel {actual}, expected {expected}"
    )]
    ChannelMismatch {
        message_id: String,
        expected: String,
        actual: String,
    },
    #[error("locate response target message {0} is not visible to viewer")]
    TargetInvisible(String),
    #[error("locate response page size is invalid: {0}")]
    InvalidPageSize(u32),
}

/// 过滤远端 context 页，确保目标和所有可见 rows 属于同一 viewer/channel 作用域。
pub fn filter_visible_remote_rows(
    rows: &[Value],
    expected_channel_id: &str,
    target_message_id: &str,
    viewer_user_id: &str,
) -> Result<Vec<Value>, LocateProjectionError> {
    let channel_id = crate::state::ChannelId::from_str(expected_channel_id).ok_or_else(|| {
        LocateProjectionError::ChannelMismatch {
            message_id: target_message_id.to_string(),
            expected: expected_channel_id.to_string(),
            actual: String::new(),
        }
    })?;
    let mut visible_rows = Vec::with_capacity(rows.len());
    let mut target_present = false;
    for row in rows {
        let message_id = row
            .get("id")
            .or_else(|| row.get("message_id"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        let actual_channel = row
            .get("channelId")
            .or_else(|| row.get("channel_id"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        if actual_channel != expected_channel_id {
            return Err(LocateProjectionError::ChannelMismatch {
                message_id: message_id.to_string(),
                expected: expected_channel_id.to_string(),
                actual: actual_channel.to_string(),
            });
        }
        let is_target =
            crate::timeline_navigation::row_matches_message_identity(row, target_message_id);
        target_present |= is_target;
        let fields = crate::ws::parser::extract_post_fields(row);
        let visible = !row_is_revoked(row)
            && crate::channel_write::post_updates_from_fields(channel_id, &fields, viewer_user_id)
                .visible;
        if is_target && !visible {
            return Err(LocateProjectionError::TargetInvisible(
                target_message_id.to_string(),
            ));
        }
        if visible {
            visible_rows.push(row.clone());
        }
    }
    if !target_present {
        return Err(LocateProjectionError::TargetMissing(
            target_message_id.to_string(),
        ));
    }
    Ok(visible_rows)
}

/// 对 Persist 后的 bounded Scan 做 channel/viewer 校验并取目标两侧的居中窗口。
pub fn normalize_durable_located_window(
    rows: Vec<Value>,
    expected_channel_id: &str,
    target_message_id: &str,
    viewer_user_id: &str,
    page_size: u32,
) -> Result<Vec<Value>, LocateProjectionError> {
    let channel_id = crate::state::ChannelId::from_str(expected_channel_id).ok_or_else(|| {
        LocateProjectionError::ChannelMismatch {
            message_id: target_message_id.to_string(),
            expected: expected_channel_id.to_string(),
            actual: String::new(),
        }
    })?;
    let page_size = crate::timeline_state::TimelinePageSize::new(page_size)
        .map_err(|_| LocateProjectionError::InvalidPageSize(page_size))?;
    let mut visible_rows = Vec::with_capacity(rows.len());
    let mut target_present = false;
    for row in rows {
        let message_id = row
            .get("id")
            .or_else(|| row.get("message_id"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        let actual_channel = row
            .get("channel_id")
            .or_else(|| row.get("channelId"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        if actual_channel != expected_channel_id {
            return Err(LocateProjectionError::ChannelMismatch {
                message_id: message_id.to_string(),
                expected: expected_channel_id.to_string(),
                actual: actual_channel.to_string(),
            });
        }
        let is_target =
            crate::timeline_navigation::row_matches_message_identity(&row, target_message_id);
        target_present |= is_target;
        let fields = crate::ws::parser::extract_post_fields(&row);
        let visible = !row_is_revoked(&row)
            && crate::channel_write::post_updates_from_fields(channel_id, &fields, viewer_user_id)
                .visible;
        if is_target && !visible {
            return Err(LocateProjectionError::TargetInvisible(
                target_message_id.to_string(),
            ));
        }
        if visible {
            visible_rows.push(row);
        }
    }
    if !target_present {
        return Err(LocateProjectionError::TargetMissing(
            target_message_id.to_string(),
        ));
    }
    visible_rows.sort_by_key(durable_row_key);
    let target_index = visible_rows
        .iter()
        .position(|row| {
            crate::timeline_navigation::row_matches_message_identity(row, target_message_id)
        })
        .ok_or_else(|| LocateProjectionError::TargetMissing(target_message_id.to_string()))?;
    let allocation = crate::timeline_state::allocate_locate_window(
        target_index,
        visible_rows.len().saturating_sub(target_index + 1),
        page_size,
    );
    let start = target_index.saturating_sub(allocation.before);
    let end = (target_index + allocation.after + 1).min(visible_rows.len());
    let shaped = crate::render_ready::shape_message_rows_for_viewer(
        &Value::Array(visible_rows[start..end].to_vec()),
        viewer_user_id,
    );
    Ok(shaped.as_array().cloned().unwrap_or_default())
}

/// 识别 durable 或 wire 行中的撤回标记，不把撤回目标重新暴露为定位结果。
fn row_is_revoked(row: &Value) -> bool {
    match row.get("revoke").or_else(|| row.get("revoked")) {
        Some(Value::Bool(value)) => *value,
        Some(Value::Number(value)) => value.as_i64().is_some_and(|value| value != 0),
        _ => false,
    }
}

/// 生成稳定的 durable 行排序键；缺失键排在结果末尾并最终由 target 校验拦截。
fn durable_row_key(row: &Value) -> (i64, String) {
    (
        row.get("create_at")
            .or_else(|| row.get("createAt"))
            .and_then(Value::as_i64)
            .unwrap_or_default(),
        row.get("temporary_id")
            .or_else(|| row.get("temporaryId"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
    )
}

/// Validate the composed `posts/get` + context authority before it becomes a timeline window.
/// The target is represented by `TimelineWindowVm.targetMessageId`, never directive payload.
pub fn normalize_located_window(
    raw_body: &[u8],
    expected_channel_id: &str,
    target_message_id: &str,
    viewer_user_id: &str,
) -> Result<Vec<serde_json::Value>, LocateProjectionError> {
    let rows = crate::older_context::extract_post_rows(raw_body);
    let shaped = crate::render_ready::shape_message_rows_for_viewer(
        &serde_json::Value::Array(rows),
        viewer_user_id,
    );
    let rows = shaped.as_array().cloned().unwrap_or_default();
    let mut target_found = false;
    for row in &rows {
        let message_id = row
            .get("msgId")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");
        let channel_id = row
            .get("channelId")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");
        if channel_id != expected_channel_id {
            return Err(LocateProjectionError::ChannelMismatch {
                message_id: message_id.to_string(),
                expected: expected_channel_id.to_string(),
                actual: channel_id.to_string(),
            });
        }
        target_found |=
            crate::timeline_navigation::row_matches_message_identity(row, target_message_id);
    }
    if !target_found {
        return Err(LocateProjectionError::TargetMissing(
            target_message_id.to_string(),
        ));
    }
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    /// 证明 locate authority 必须包含目标且所有 rows 属于同一 channel。
    fn located_window_requires_exact_target_and_single_channel() {
        let raw = br#"{"data":[
            {"id":"before","channelId":"c1","userId":"u1","message":"before"},
            {"id":"target","channelId":"c1","userId":"u2","message":"target"}
        ]}"#;
        let rows = normalize_located_window(raw, "c1", "target", "u1").unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(
            normalize_located_window(raw, "c1", "missing", "u1"),
            Err(LocateProjectionError::TargetMissing("missing".to_string()))
        );

        let cross_channel = br#"{"data":[{"id":"target","channelId":"c2","userId":"u2"}]}"#;
        assert!(matches!(
            normalize_located_window(cross_channel, "c1", "target", "u1"),
            Err(LocateProjectionError::ChannelMismatch { .. })
        ));
    }
}
