//! Webhook 分发配置 outbound（#35-37，session 鉴权，根路由 webhook_dispatcher.go:13-15）。
//!
//! 响应走裸 `map[string]interface{}{"success","message"}`（**非 dto.CommonRes**，#35-37 特例）——
//! 属**响应态**，outbound 只 build 请求体，响应信封解析在 driver/前端（文档断言锚边界）。
//! #36 getWebhookConfig 无请求体（GET）；method 差异属 driver 接缝 P6。

use serde_json::{json, Value};

use crate::error::ImError;
use crate::outbound::registry::require_str;

/// 取必填字符串数组（非空、元素全 string）。供 botUserIds/channelFilter 复用。
fn str_array(args: &Value, key: &str) -> Option<Value> {
    args.get(key)
        .filter(|v| v.as_array().is_some_and(|a| a.iter().all(Value::is_string)))
        .cloned()
}

// #35 POST /webhook/config — WebhookConfigRequest{targetUrl,secret,botUserIds:[],triggerMode,channelFilter?}.
bot_cmd!(
    SetWebhookConfigCommand,
    SET_WEBHOOK_CONFIG_REG,
    "im_webhook_config_set",
    |args, cmd| {
        let target_url = require_str(args, "target_url", cmd)?;
        let secret = require_str(args, "secret", cmd)?;
        let trigger_mode = require_str(args, "trigger_mode", cmd)?;
        let bot_user_ids = str_array(args, "bot_user_ids")
            .ok_or_else(|| ImError::Parse(format!("{cmd}: 缺/坏 bot_user_ids（字符串数组）")))?;
        let mut b = json!({
            "targetUrl": target_url,
            "secret": secret,
            "botUserIds": bot_user_ids,
            "triggerMode": trigger_mode,
        });
        if let Some(channel_filter) = str_array(args, "channel_filter") {
            b["channelFilter"] = channel_filter;
        }
        Ok(("webhook/config", b))
    }
);

// #36 GET /webhook/config — 无请求体（响应裸 map，configured?）。
bot_cmd!(
    GetWebhookConfigCommand,
    GET_WEBHOOK_CONFIG_REG,
    "im_webhook_config_get",
    |_args, _cmd| { Ok(("webhook/config", json!({}))) }
);

// #37 POST /webhook/test — WebhookTestRequest{message?}（出站 OpenClaw）。message 可选。
bot_cmd!(
    TestWebhookCommand,
    TEST_WEBHOOK_REG,
    "im_webhook_test",
    |args, _cmd| {
        let mut b = json!({});
        if let Some(message) = args.get("message").and_then(Value::as_str) {
            b["message"] = json!(message);
        }
        Ok(("webhook/test", b))
    }
);
