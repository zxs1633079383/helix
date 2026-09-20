//! Bot-agent **调用** outbound（#22-27，**bot-token 鉴权**，bot_agent.go:52-62 agentRouter）。
//!
//! ## C6 / HX-C001 铁律（本文件 `bot_call_cmd!`/`bot_call_dyn_cmd!`，auth_kind=Bot）
//! 这 6 命令是现网 bot-token 鉴权轴（BotAgentTokenAuth 中间件，凭据形态见后端 bot_agent.go）。
//! 本 build 层**绝不**拼任何 bot 凭据 token——只标 `X-Auth-Kind: bot` 意图，真凭据由 driver
//! 横切层注入（C6 闸门：本 crate 不出现 token 字面量 / 鉴权头名）。
//!
//! 含 path-param GET（#25/26/27 `{team_id}`/`{user_id}` + #25 query `?page=&per_page=`）+ POST send。
//! Task1 起 method 由 registry 按命令名集中产出。send body = BotAgentSendMessageRequest（direct/channel
//! 共用，camelCase）。

use serde_json::{json, Value};

use super::path_seg;
use crate::error::ImError;
use crate::outbound::registry::require_str;

// #22 GET /bot-agent/info — 无 App 调用，从 ctx config 取（响应裸 map{id,username,displayName}，响应态）。
bot_call_cmd!(
    read BotAgentInfoCommand,
    BOT_AGENT_INFO_REG,
    "im_bot_agent_info",
    |_args, _cmd| { Ok(("bot-agent/info", json!({}))) }
);

// #23 POST /bot-agent/messages/direct — BotAgentSendMessageRequest{teamId,userId,channelId?,message,
// createChannel?,props?,userSnapshot?}（校验 teamId/userId/message 非空，direct 必给 userId）。
bot_call_cmd!(
    BotAgentSendDirectCommand,
    BOT_AGENT_SEND_DIRECT_REG,
    "im_bot_agent_send_direct",
    |args, cmd| {
        let mut b = send_message_common(args, cmd)?;
        b["userId"] = json!(require_str(args, "user_id", cmd)?);
        Ok(("bot-agent/messages/direct", b))
    }
);

// #24 POST /bot-agent/messages/channel — 同 BotAgentSendMessageRequest（校验 teamId/channelId/message）。
bot_call_cmd!(
    BotAgentSendChannelCommand,
    BOT_AGENT_SEND_CHANNEL_REG,
    "im_bot_agent_send_channel",
    |args, cmd| {
        let mut b = send_message_common(args, cmd)?;
        b["channelId"] = json!(require_str(args, "channel_id", cmd)?);
        Ok(("bot-agent/messages/channel", b))
    }
);

// #25 GET /bot-agent/teams/{team_id}/members?page=&per_page= — path + query param。
bot_call_dyn_cmd!(
    read BotAgentTeamMembersCommand,
    BOT_AGENT_TEAM_MEMBERS_REG,
    "im_bot_agent_team_members",
    "bot-agent/teams",
    |args, cmd| {
        let team_id = path_seg(args, "team_id", cmd)?;
        let page = args.get("page").and_then(Value::as_i64).unwrap_or(0);
        let per_page = args.get("per_page").and_then(Value::as_i64).unwrap_or(0);
        Ok(format!(
            "bot-agent/teams/{team_id}/members?page={page}&per_page={per_page}"
        ))
    },
    |_args, _cmd| Ok(json!({}))
);

// #26 GET /bot-agent/teams/{team_id}/users/{user_id} — 双 path param。
bot_call_dyn_cmd!(
    read BotAgentGetUserCommand,
    BOT_AGENT_GET_USER_REG,
    "im_bot_agent_get_user",
    "bot-agent/teams",
    |args, cmd| Ok(format!(
        "bot-agent/teams/{}/users/{}",
        path_seg(args, "team_id", cmd)?,
        path_seg(args, "user_id", cmd)?
    )),
    |_args, _cmd| Ok(json!({}))
);

// #27 GET /bot-agent/teams/{team_id}/channel — path param（默认 channel）。
bot_call_dyn_cmd!(
    read BotAgentTeamChannelCommand,
    BOT_AGENT_TEAM_CHANNEL_REG,
    "im_bot_agent_team_channel",
    "bot-agent/teams",
    |args, cmd| Ok(format!(
        "bot-agent/teams/{}/channel",
        path_seg(args, "team_id", cmd)?
    )),
    |_args, _cmd| Ok(json!({}))
);

/// BotAgentSendMessageRequest 公共字段（teamId/message 必填 + createChannel?/props?/userSnapshot?）。
/// direct/channel 各自再补 userId/channelId（见 #23/#24）。
fn send_message_common(args: &Value, cmd: &str) -> Result<Value, ImError> {
    let team_id = require_str(args, "team_id", cmd)?;
    let message = require_str(args, "message", cmd)?;
    let mut b = json!({ "teamId": team_id, "message": message });
    if let Some(create_channel) = args.get("create_channel").and_then(Value::as_bool) {
        b["createChannel"] = json!(create_channel);
    }
    if let Some(props) = args.get("props").filter(|v| v.is_object()) {
        b["props"] = props.clone();
    }
    if let Some(snapshot) = args.get("user_snapshot").filter(|v| v.is_object()) {
        b["userSnapshot"] = snapshot.clone();
    }
    Ok(crate::message_identity::sanitize(b))
}
