//! `parser::extract_post_fields` 的 props 合并 helper（从 parser.rs 外提，收口 parser.rs 行数基线）。

use serde_json::Value;

/// 合并 post.props 与顶层 `quickReply`（UC-1.8 快捷回复 emoji 寄生 props.quickReply）。
///
/// 服务端 `post.ToMap()`（copy_post.go:72）把 emoji 反应作**顶层** `quickReply` 字段下发
/// （`[{emoji, userIds:[...]}]`·post_update WS echo），不在 post.props 内。投影 fat 集只携 props
/// （13 键固定·projection-schema），故把顶层 quickReply **合并进 props.quickReply**——让反应数据搭
/// props 顺风车进投影（契约 13 键不变·quickReply 寄生 props），前端从 props.quickReply 渲染
/// data-reactions。非 quickReply 类 post echo 顶层无此字段 → props 原样不动（零副作用）。
///
/// 返回 props 的 JSON 字符串（空对象/null → 空串，对齐原 parser.rs 行为）。
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
