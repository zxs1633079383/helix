//! `ws::parser::extract_post_fields` 的 props 合并 helper（从 core.rs 外提，收口 core.rs 行数基线）。

use serde_json::Value;

/// 合并 post.props 与顶层 `quickReply`（UC-1.8 快捷回复 emoji 寄生 props.quickReply）。
///
/// 服务端 `post.ToMap()`（copy_post.go:72）把 emoji 反应作**顶层** `quickReply` 字段下发
/// （`[{emoji, userIds:[...]}]`·post_update WS echo），不在 post.props 内。投影 fat 集只携 props
/// （13 键固定·projection-schema），故把顶层 quickReply **合并进 props.quickReply**——让反应数据搭
/// props 顺风车进投影（契约 13 键不变·quickReply 寄生 props），前端从 props.quickReply 渲染
/// data-reactions。非 quickReply 类 post echo 顶层无此字段 → props 原样不动（零副作用）。
///
/// 返回 props 的 JSON 字符串：缺省 props（无键）→ `"{}"`；显式 `null` props → 空串；
/// 其余原样 stringify（对齐参考实现 e1a9c99 / 5db0d15）。真实 post echo 恒带 props
/// （`channel_event_seq` 等），缺省分支仅理论边界。
pub(crate) fn merge_props_with_quickreply(post: &Value, root: &Value) -> String {
    let base = post
        .get("props")
        .or_else(|| root.get("props"))
        .cloned()
        .unwrap_or_else(|| Value::Object(serde_json::Map::new()));
    let quick_reply = post
        .get("quickReply")
        .or_else(|| root.get("quickReply"))
        .filter(|v| !v.is_null());
    match (base, quick_reply) {
        (Value::Object(mut map), Some(qr)) => {
            map.insert("quickReply".to_string(), qr.clone());
            Value::Object(map).to_string()
        }
        (base, _) => {
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
    use super::merge_props_with_quickreply;
    use serde_json::json;

    /// UC-1.8：顶层 quickReply 合并进 props.quickReply（反应数据搭 props 顺风车进投影）。
    #[test]
    fn top_level_quickreply_merges_into_props() {
        let post = json!({
            "props": { "channel_event_seq": 50 },
            "quickReply": [{ "emoji": "👍", "userIds": ["u1", "u2"] }],
        });
        let out = merge_props_with_quickreply(&post, &json!({}));
        let parsed: serde_json::Value = serde_json::from_str(&out).expect("props 应为合法 JSON");
        assert_eq!(parsed["channel_event_seq"], 50, "原 props 业务键保留");
        assert_eq!(
            parsed["quickReply"][0]["emoji"], "👍",
            "顶层 quickReply 应合并进 props.quickReply"
        );
        assert_eq!(parsed["quickReply"][0]["userIds"][1], "u2");
    }

    /// quickReply 缺省 post 在根上携带时也应被拾取（root fallback）。
    #[test]
    fn root_level_quickreply_is_picked_up() {
        let post = json!({ "props": {} });
        let root = json!({ "quickReply": [{ "emoji": "🎉", "userIds": ["u9"] }] });
        let out = merge_props_with_quickreply(&post, &root);
        let parsed: serde_json::Value = serde_json::from_str(&out).expect("props 应为合法 JSON");
        assert_eq!(parsed["quickReply"][0]["emoji"], "🎉");
    }

    /// 非 quickReply 类 post：props 原样不动（零副作用），无 quickReply 键。
    #[test]
    fn plain_post_without_quickreply_keeps_props_untouched() {
        let post = json!({ "props": { "channel_event_seq": 7 } });
        let out = merge_props_with_quickreply(&post, &json!({}));
        let parsed: serde_json::Value = serde_json::from_str(&out).expect("props 应为合法 JSON");
        assert_eq!(parsed["channel_event_seq"], 7);
        assert!(parsed.get("quickReply").is_none(), "无反应数据时不应注入 quickReply 键");
    }

    /// 缺省 props（无键）+ 无 quickReply → `"{}"`（参考实现 e1a9c99/5db0d15 行为）；
    /// 显式 `null` props → 空串。真实 post echo 恒带 props，此为理论边界守护。
    #[test]
    fn missing_props_yields_empty_object_explicit_null_yields_empty_string() {
        let missing = merge_props_with_quickreply(&json!({}), &json!({}));
        assert_eq!(missing, "{}", "props 缺省 + 无 quickReply → 空对象串");
        let explicit_null = merge_props_with_quickreply(&json!({ "props": null }), &json!({}));
        assert_eq!(explicit_null, "", "显式 null props → 空串");
    }
}
