//! S5（issue #54·C013）：dialogList 行 render-ready 升级辅助。
//!
//! 把一行原始 channel 扫描结果（DB 列·snake_case）升成 render-ready CL 行——前端壳零
//! normalize/零 ++，1:1 绑定 data-*。从 query.rs 外提（守 structure-gate 单文件 ≤300 行）。

use serde_json::Value;

/// 把一行原始 channel 扫描结果升成 render-ready CL 行。
///
/// 保留原始列（向后兼容旧消费方），叠加 8 个派生终态键：
/// displayName / notice(归一) / isTop(bool) / createAt / unread(终值) / mention(bool) /
/// lastMessage(预览) / urgent(bool)。非对象行 → 原样返回（零信任不 panic）。
pub(crate) fn render_ready_dialog_row(row: Value) -> Value {
    let Value::Object(mut obj) = row else {
        return row;
    };
    // displayName ← display_name（render-ready·前端不再 snake 抽）。
    if let Some(Value::String(s)) = obj.get("display_name") {
        let s = s.clone();
        obj.insert("displayName".into(), Value::String(s));
    }
    // notice ← 归一（notice 列可能是 JSON `{"text":".."}` 或纯文本·空→null·前端删 normalizeNotice）。
    obj.insert("notice".into(), normalize_notice_value(obj.get("notice")));
    // isTop ← is_top（INTEGER/bool/字符串 → bool·前端删 normalizeIsTop）。
    obj.insert("isTop".into(), Value::Bool(truthy(obj.get("is_top"))));
    // createAt ← create_at（毫秒·CL 升序排序键·snake→camel 归一）。
    if let Some(n) = as_i64(obj.get("create_at")) {
        obj.insert("createAt".into(), Value::from(n));
    }
    // unread ← unread_count 终值（前端不再 ++·badge 直绑）。
    obj.insert(
        "unread".into(),
        Value::from(as_i64(obj.get("unread_count")).unwrap_or(0)),
    );
    // mention ← mention_count>0 || mention_count_root>0（艾特 badge·bool 终值）。
    let mention = as_i64(obj.get("mention_count")).unwrap_or(0) > 0
        || as_i64(obj.get("mention_count_root")).unwrap_or(0) > 0;
    obj.insert("mention".into(), Value::Bool(mention));
    // lastMessage / urgent ← last_post（整帧 post JSON 或内容兜底字符串·解析取预览 + 加急标）。
    let (last_message, urgent) = derive_last_post(obj.get("last_post"));
    obj.insert("lastMessage".into(), last_message);
    obj.insert("urgent".into(), Value::Bool(urgent));
    Value::Object(obj)
}

/// notice 列归一：JSON `{"text":".."}` → text；纯文本 → 原样；空/缺/非串 → null。
fn normalize_notice_value(v: Option<&Value>) -> Value {
    let Some(Value::String(s)) = v else {
        return Value::Null;
    };
    if s.is_empty() {
        return Value::Null;
    }
    // 形如 {"text":".."} → 抽 text；否则原样透传字符串。
    if let Ok(Value::Object(m)) = serde_json::from_str::<Value>(s) {
        if let Some(Value::String(t)) = m.get("text") {
            return if t.is_empty() {
                Value::Null
            } else {
                Value::String(t.clone())
            };
        }
    }
    Value::String(s.clone())
}

/// last_post 解析：整帧 post JSON → (message 预览, urgent 标)；纯字符串兜底 → (该串, false)。
fn derive_last_post(v: Option<&Value>) -> (Value, bool) {
    let Some(Value::String(s)) = v else {
        return (Value::Null, false);
    };
    if s.is_empty() {
        return (Value::Null, false);
    }
    if let Ok(Value::Object(m)) = serde_json::from_str::<Value>(s) {
        // message 预览：message ?? content ?? text。
        let preview = ["message", "content", "text"]
            .iter()
            .find_map(|k| match m.get(*k) {
                Some(Value::String(t)) if !t.is_empty() => Some(Value::String(t.clone())),
                _ => None,
            })
            .unwrap_or(Value::Null);
        // 加急标：顶层 isUrgent/isExpedite 或 props.expedite/props.urgent 任一「存在且非空」。
        // 注：expedite 常为目标 id 串/映射（如 "m1" / {...}），非空即加急（非严格 1/true 真值）。
        let props = m.get("props");
        let urgent = nonempty(m.get("isUrgent"))
            || nonempty(m.get("isExpedite"))
            || props
                .map(|p| nonempty(p.get("expedite")) || nonempty(p.get("urgent")))
                .unwrap_or(false);
        return (preview, urgent);
    }
    // 非 JSON：内容兜底字符串作预览（sync 路径 last_post 用消息内容兜底）。
    (Value::String(s.clone()), false)
}

/// 宽松真值判定（INTEGER!=0 / bool / "1"/"true" / 非空对象-数组）→ bool。
fn truthy(v: Option<&Value>) -> bool {
    match v {
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_i64().map(|i| i != 0).unwrap_or(false),
        Some(Value::String(s)) => s == "1" || s.eq_ignore_ascii_case("true"),
        Some(Value::Object(m)) => !m.is_empty(),
        Some(Value::Array(a)) => !a.is_empty(),
        _ => false,
    }
}

/// 「存在且非空」判定——用于 expedite/urgent 这类「非空即命中」语义字段。
/// 与 truthy 区别：非空字符串（如目标 id 串 "m1"）/ 非零数 / true / 非空对象-数组 → true。
fn nonempty(v: Option<&Value>) -> bool {
    match v {
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_i64().map(|i| i != 0).unwrap_or(true),
        Some(Value::String(s)) => !s.is_empty(),
        Some(Value::Object(m)) => !m.is_empty(),
        Some(Value::Array(a)) => !a.is_empty(),
        _ => false,
    }
}

/// 宽松整数抽取（Number / 数字字符串）→ i64。
fn as_i64(v: Option<&Value>) -> Option<i64> {
    match v {
        Some(Value::Number(n)) => n.as_i64().or_else(|| n.as_f64().map(|f| f as i64)),
        Some(Value::String(s)) => s.parse::<i64>().ok(),
        _ => None,
    }
}
