//! AI Agent 会话编排 outbound（#28-32，session 鉴权，agents/* agent.go:14-18）。
//!
//! #32 agentCallback 落 post（CreateCsesPost）后 WS 推 `posted`，且 **内存构造 post 不推 cursor**
//! （gap-bot-agent.md:44，与 mattermost 一致不写 channel_event）——属响应/WS 推送态，outbound 只 build
//! 请求体（文档断言锚边界）。**排除** timeline/cost（#33/34 D7 废弃，无 live handler，不注册）。

use serde_json::{json, Value};

use crate::error::ImError;
use crate::outbound::registry::require_str;

// #28 POST /agents/create — CreateAgentRequest{name,type,description?,teamId,ownerUserId?}.
bot_cmd!(
    AgentCreateCommand,
    AGENT_CREATE_REG,
    "im_agent_create",
    |args, cmd| {
        let name = require_str(args, "name", cmd)?;
        let typ = require_str(args, "type", cmd)?;
        let team_id = require_str(args, "team_id", cmd)?;
        let mut b = json!({ "name": name, "type": typ, "teamId": team_id });
        if let Some(description) = args.get("description").and_then(Value::as_str) {
            b["description"] = json!(description);
        }
        if let Some(owner_user_id) = args.get("owner_user_id").and_then(Value::as_str) {
            b["ownerUserId"] = json!(owner_user_id);
        }
        Ok(("agents/create", b))
    }
);

// #29 POST /agents/join-channel — JoinAgentChannelRequest{type,channelId,teamId}.
bot_cmd!(
    AgentJoinChannelCommand,
    AGENT_JOIN_CHANNEL_REG,
    "im_agent_join_channel",
    |args, cmd| {
        let typ = require_str(args, "type", cmd)?;
        let channel_id = require_str(args, "channel_id", cmd)?;
        let team_id = require_str(args, "team_id", cmd)?;
        Ok((
            "agents/join-channel",
            json!({ "type": typ, "channelId": channel_id, "teamId": team_id }),
        ))
    }
);

// #30 POST /agents/start-session — StartAgentSessionRequest{sessionId?,taskName?,teamId,starterUserId?,
// resultChannelId,backstageChannelId?,agentTypes:[]}.
bot_cmd!(
    AgentStartSessionCommand,
    AGENT_START_SESSION_REG,
    "im_agent_start_session",
    |args, cmd| {
        let team_id = require_str(args, "team_id", cmd)?;
        let result_channel_id = require_str(args, "result_channel_id", cmd)?;
        let agent_types = require_str_array(args, "agent_types", cmd)?;
        let mut b = json!({
            "teamId": team_id,
            "resultChannelId": result_channel_id,
            "agentTypes": agent_types,
        });
        for (src, dst) in [
            ("session_id", "sessionId"),
            ("task_name", "taskName"),
            ("starter_user_id", "starterUserId"),
            ("backstage_channel_id", "backstageChannelId"),
        ] {
            if let Some(v) = args.get(src).and_then(Value::as_str) {
                b[dst] = json!(v);
            }
        }
        Ok(("agents/start-session", b))
    }
);

// #31 POST /agents/send — SendAgentMessageRequest{sessionId,agentType,text,conversationType?,channelId?,userId?}.
bot_cmd!(
    AgentSendCommand,
    AGENT_SEND_REG,
    "im_agent_send",
    |args, cmd| {
        let session_id = require_str(args, "session_id", cmd)?;
        let agent_type = require_str(args, "agent_type", cmd)?;
        let text = require_str(args, "text", cmd)?;
        let mut b = json!({ "sessionId": session_id, "agentType": agent_type, "text": text });
        for (src, dst) in [
            ("conversation_type", "conversationType"),
            ("channel_id", "channelId"),
            ("user_id", "userId"),
        ] {
            if let Some(v) = args.get(src).and_then(Value::as_str) {
                b[dst] = json!(v);
            }
        }
        Ok(("agents/send", b))
    }
);

// #32 POST /agents/callback — AgentCallbackRequest{messageId,...,text,usage{...},routing{...}}.
// 内存构造 post 不推 cursor（响应/WS 态，见 mod 文档）。routing/usage 内嵌对象整体透传。
bot_cmd!(
    AgentCallbackCommand,
    AGENT_CALLBACK_REG,
    "im_agent_callback",
    |args, cmd| {
        let message_id = require_str(args, "message_id", cmd)?;
        let text = require_str(args, "text", cmd)?;
        let mut b = json!({ "messageId": message_id, "text": text });
        for (src, dst) in [
            ("session_id", "sessionId"),
            ("trace_id", "traceId"),
            ("span_id", "spanId"),
            ("parent_span_id", "parentSpanId"),
            ("agent_type", "agentType"),
            ("bot_user_id", "botUserId"),
            ("stage", "stage"),
            ("to", "to"),
            ("reply_to_id", "replyToId"),
            ("thread_id", "threadId"),
        ] {
            if let Some(v) = args.get(src).and_then(Value::as_str) {
                b[dst] = json!(v);
            }
        }
        if let Some(commands) = args.get("commands").filter(|v| v.is_array()) {
            b["commands"] = commands.clone();
        }
        if let Some(usage) = args.get("usage").filter(|v| v.is_object()) {
            b["usage"] = usage.clone();
        }
        if let Some(routing) = args.get("routing").filter(|v| v.is_object()) {
            b["routing"] = routing.clone();
        }
        Ok(("agents/callback", b))
    }
);

/// 取必填字符串数组（非空、元素全 string）。供 agentTypes 类字段复用（边界零信任）。
fn require_str_array(args: &Value, key: &str, cmd: &str) -> Result<Value, ImError> {
    args.get(key)
        .and_then(Value::as_array)
        .filter(|a| !a.is_empty() && a.iter().all(Value::is_string))
        .cloned()
        .map(Value::Array)
        .ok_or_else(|| ImError::Parse(format!("{cmd}: 缺/空 {key}（非空字符串数组）")))
}
