//! Bot gateway outbound（#1 createBot / #2 botCallback，session 鉴权，posts/* 子路由）。
//!
//! 真源 bot.go:15/18 + entity BotMessageRequest / BotCallbackRequest（全 camelCase）。
//! #2 botCallback 落 post（CreateCsesPost）后 WS 推 `posted`——属服务端响应/WS 推送态，
//! 且 **内存构造 post 不推 cursor**（gap-bot-agent.md:14，与 mattermost 一致不写 channel_event）；
//! outbound 只 build **请求体**，cursor 语义不在此裁决（文档断言锚边界，见 tests）。

use serde_json::{json, Value};

use crate::error::ImError;
use crate::outbound::registry::require_str;

// #1 POST /api/cses/posts/createBot — BotMessageRequest{sessionId,userId?,channelId?,conversationType?,text}.
// userId 空时服务端回填 session（gateway_bot.go）；helix 仅下发显式字段，不臆造空串占位。
bot_cmd!(
    CreateBotCommand,
    CREATE_BOT_REG,
    "im_create_bot",
    |args, cmd| {
        let session_id = require_str(args, "session_id", cmd)?;
        let text = require_str(args, "text", cmd)?;
        let mut b = json!({ "sessionId": session_id, "text": text });
        if let Some(user_id) = args.get("user_id").and_then(Value::as_str) {
            b["userId"] = json!(user_id);
        }
        if let Some(channel_id) = args.get("channel_id").and_then(Value::as_str) {
            b["channelId"] = json!(channel_id);
        }
        if let Some(ct) = args.get("conversation_type").and_then(Value::as_str) {
            b["conversationType"] = json!(ct);
        }
        Ok(("posts/createBot", b))
    }
);

// #2 POST /api/cses/posts/botCallback — BotCallbackRequest{messageId,timestamp,to,text,mediaUrl?,
// replyToId?,threadId?,routing{channelId?,userId?,sessionId?}}（entity BotCallbackRequest verbatim）。
// 内嵌 routing 对象整体透传（前端给完整 wire 对象，camelCase）。
bot_cmd!(
    BotCallbackCommand,
    BOT_CALLBACK_REG,
    "im_bot_callback",
    |args, cmd| {
        let message_id = require_str(args, "message_id", cmd)?;
        let text = require_str(args, "text", cmd)?;
        let to = require_str(args, "to", cmd)?;
        let timestamp = args
            .get("timestamp")
            .and_then(Value::as_i64)
            .ok_or_else(|| ImError::Parse(format!("{cmd}: 缺/坏 timestamp（int64）")))?;
        let routing = args
            .get("routing")
            .filter(|v| v.is_object())
            .cloned()
            .unwrap_or_else(|| json!({}));
        let mut b = json!({
            "messageId": message_id,
            "timestamp": timestamp,
            "to": to,
            "text": text,
            "routing": routing,
        });
        for (src, dst) in [
            ("media_url", "mediaUrl"),
            ("reply_to_id", "replyToId"),
            ("thread_id", "threadId"),
        ] {
            if let Some(v) = args.get(src).and_then(Value::as_str) {
                b[dst] = json!(v);
            }
        }
        Ok(("posts/botCallback", b))
    }
);
