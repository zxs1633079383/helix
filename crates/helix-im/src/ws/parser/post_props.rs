//! `ws::parser::extract_post_fields` 的 props 规范化 helper。

use serde_json::Value;

/// 生成独立于 quickReply durable 列的 props JSON 文本。
pub(crate) fn props_with_event_seq(post: &Value, root: &Value, event_seq: Option<u64>) -> String {
    let base = post
        .get("props")
        .or_else(|| root.get("props"))
        .cloned()
        .unwrap_or_else(|| Value::Object(serde_json::Map::new()));
    match base {
        Value::Object(mut map) => {
            map.remove("quickReply");
            map.remove("quick_reply");
            if let Some(seq) = event_seq {
                map.insert("channel_event_seq".to_string(), Value::from(seq));
            }
            Value::Object(map).to_string()
        }
        base => {
            if base.is_null() {
                String::new()
            } else {
                base.to_string()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::props_with_event_seq;
    use serde_json::json;

    /// G-06：顶层 quickReply 不再寄生到 props。
    #[test]
    fn top_level_quickreply_stays_outside_props() {
        let post = json!({
            "props": { "channel_event_seq": 50 },
            "quickReply": [{ "emoji": "👍", "userIds": ["u1", "u2"] }],
        });
        let out = props_with_event_seq(&post, &json!({}), None);
        let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&out) else {
            panic!("props 应为合法 JSON");
        };
        assert_eq!(parsed["channel_event_seq"], 50, "原 props 业务键保留");
        assert!(parsed.get("quickReply").is_none());
    }

    /// G-06：root fallback 的 quickReply 同样不污染 props。
    #[test]
    fn root_level_quickreply_stays_outside_props() {
        let post = json!({ "props": {} });
        let root = json!({ "quickReply": [{ "emoji": "🎉", "userIds": ["u9"] }] });
        let out = props_with_event_seq(&post, &root, None);
        let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&out) else {
            panic!("props 应为合法 JSON");
        };
        assert!(parsed.get("quickReply").is_none());
    }

    /// 运行面旧帧把 reaction 寄生进 props 时也必须剥离，避免双权威列。
    #[test]
    fn nested_quickreply_is_removed_from_props() {
        let post = json!({
            "props": {
                "quickReply": [{ "emoji": "thumb", "userIds": ["member-1"] }],
                "businessKey": "kept"
            }
        });
        let parsed: serde_json::Value =
            serde_json::from_str(&props_with_event_seq(&post, &json!({}), None)).unwrap();

        assert!(parsed.get("quickReply").is_none());
        assert_eq!(parsed["businessKey"], "kept");
    }

    /// 非 quickReply 类 post：props 原样不动（零副作用），无 quickReply 键。
    #[test]
    fn plain_post_without_quickreply_keeps_props_untouched() {
        let post = json!({ "props": { "channel_event_seq": 7 } });
        let out = props_with_event_seq(&post, &json!({}), None);
        let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&out) else {
            panic!("props 应为合法 JSON");
        };
        assert_eq!(parsed["channel_event_seq"], 7);
        assert!(
            parsed.get("quickReply").is_none(),
            "无反应数据时不应注入 quickReply 键"
        );
    }

    /// 缺省 props（无键）+ 无 quickReply → `"{}"`（参考实现 e1a9c99/5db0d15 行为）；
    /// 显式 `null` props → 空串。真实 post echo 恒带 props，此为理论边界守护。
    #[test]
    fn missing_props_yields_empty_object_explicit_null_yields_empty_string() {
        let missing = props_with_event_seq(&json!({}), &json!({}), None);
        assert_eq!(missing, "{}", "props 缺省 + 无 quickReply → 空对象串");
        let explicit_null = props_with_event_seq(&json!({ "props": null }), &json!({}), None);
        assert_eq!(explicit_null, "", "显式 null props → 空串");
    }

    /// 受信 event_seq 仍进入 props，供未迁移投影继续对账。
    #[test]
    fn sequenced_live_post_persists_event_seq_inside_props() {
        let post = json!({"props": {}, "message": "live"});
        let out = props_with_event_seq(&post, &json!({}), Some(27));
        let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&out) else {
            panic!("props 应为合法 JSON");
        };
        assert_eq!(parsed["channel_event_seq"], 27);
    }
}
