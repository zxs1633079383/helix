//! G-11 消息定位的 render-ready 投影。

use bytes::Bytes;
use helix_core::effect::{DomainEventBytes, Effect};

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
}
