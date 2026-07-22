//! G-11 消息定位的 render-ready 投影。

use bytes::Bytes;
use helix_core::effect::{DomainEventBytes, Effect};
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
        target_found |= message_id == target_message_id;
    }
    if !target_found {
        return Err(LocateProjectionError::TargetMissing(
            target_message_id.to_string(),
        ));
    }
    Ok(rows)
}

/// Reveal is an acknowledgement-time UI semantic; the committed VM owns the target identity.
pub fn reveal_directive(directive_id: impl Into<String>) -> crate::interaction_frame::UiDirective {
    crate::interaction_frame::UiDirective {
        directive_id: directive_id.into(),
        kind: "revealMessage".to_string(),
    }
}

/// 把 `posts/get` 的数组或 `{data:[...]}` 回包整形成一条 highlighted 消息投影。
pub fn emit_located_post(raw_body: &[u8]) -> Option<Effect> {
    let rows = crate::older_context::extract_post_rows(raw_body);
    let shaped = crate::render_ready::shape_message_rows(&serde_json::Value::Array(rows));
    let mut message = shaped.as_array()?.first()?.clone();
    message
        .as_object_mut()?
        .insert("highlighted".to_string(), serde_json::Value::Bool(true));
    let payload = serde_json::json!({ "event": "im:post:located", "data": message });
    Some(Effect::Emit {
        event: DomainEventBytes(Bytes::from(
            serde_json::to_vec(&payload).expect("located projection shape must serialize"),
        )),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shapes_posts_get_data_as_highlighted_message() {
        let effect = emit_located_post(
            br#"{"data":[{"id":"p1","channelId":"c1","userId":"u1","type":"TEXT","message":"target"}]}"#,
        )
        .unwrap();
        let Effect::Emit { event } = effect else {
            panic!("must emit")
        };
        let payload: serde_json::Value = serde_json::from_slice(event.0.as_ref()).unwrap();
        assert_eq!(payload["event"], "im:post:located");
        assert_eq!(payload["data"]["id"], "p1");
        assert_eq!(payload["data"]["highlighted"], true);
    }

    #[test]
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

    #[test]
    fn reveal_directive_contains_no_target_payload() {
        let directive = reveal_directive("reveal-1");
        assert_eq!(
            serde_json::to_value(directive).unwrap(),
            serde_json::json!({"directiveId":"reveal-1","kind":"revealMessage"})
        );
    }
}
