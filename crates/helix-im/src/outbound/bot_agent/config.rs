//! Bot-agent config + channel 管理 outbound（#12-21，session 鉴权，bot-agent/* bot_agent.go:25-46）。
//!
//! 含 path-param（`{botUserId}`/`{channelId}`，mux.Vars）+ GET/DELETE method——`bot_dyn_cmd!` 的
//! `path_override` 把 path 段插进 path（endpoint 对齐现网），method 差异属 driver 接缝 P6。
//! #12 config req/resp 直接是 ent.BotAgentConfig（gap:24 未抽 DTO 技术债，透传整对象）。

use serde_json::{json, Value};

use super::path_seg;
use crate::error::ImError;
use crate::outbound::registry::require_str;

// #12 POST /bot-agent/config — body = ent.BotAgentConfig 整对象（校验 botUserId 非空）。透传 config 对象。
bot_cmd!(
    BotAgentCreateConfigCommand,
    BOT_AGENT_CREATE_CONFIG_REG,
    "im_bot_agent_config_create",
    |args, cmd| {
        let config = args
            .get("config")
            .filter(|v| v.is_object())
            .cloned()
            .ok_or_else(|| {
                ImError::Parse(format!("{cmd}: 缺/坏 config（ent.BotAgentConfig 对象）"))
            })?;
        if config
            .get("botUserId")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .is_none()
        {
            return Err(ImError::Parse(format!("{cmd}: config.botUserId 必填")));
        }
        Ok(("bot-agent/config", config))
    }
);

// #13 GET /bot-agent/config/{botUserId} — path param。
bot_dyn_cmd!(
    read BotAgentGetConfigCommand,
    BOT_AGENT_GET_CONFIG_REG,
    "im_bot_agent_config_get",
    "bot-agent/config",
    |args, cmd| Ok(format!(
        "bot-agent/config/{}",
        path_seg(args, "bot_user_id", cmd)?
    )),
    |_args, _cmd| Ok(json!({}))
);

// #14 GET /bot-agent/configs/enabled — 无参。
bot_cmd!(
    read BotAgentEnabledConfigsCommand,
    BOT_AGENT_ENABLED_CONFIGS_REG,
    "im_bot_agent_configs_enabled",
    |_args, _cmd| { Ok(("bot-agent/configs/enabled", json!({}))) }
);

// #15 DELETE /bot-agent/config/{botUserId} — teamID=session.CompanyId（服务端注入），path param。
bot_dyn_cmd!(
    BotAgentDeleteConfigCommand,
    BOT_AGENT_DELETE_CONFIG_REG,
    "im_bot_agent_config_delete",
    "bot-agent/config",
    |args, cmd| Ok(format!(
        "bot-agent/config/{}",
        path_seg(args, "bot_user_id", cmd)?
    )),
    |_args, _cmd| Ok(json!({}))
);

// #16 POST /bot-agent/config/{botUserId}/regenerate-token — path param。
bot_dyn_cmd!(
    BotAgentRegenTokenCommand,
    BOT_AGENT_REGEN_TOKEN_REG,
    "im_bot_agent_regenerate_token",
    "bot-agent/config",
    |args, cmd| Ok(format!(
        "bot-agent/config/{}/regenerate-token",
        path_seg(args, "bot_user_id", cmd)?
    )),
    |_args, _cmd| Ok(json!({}))
);

// #17 POST /bot-agent/config/{botUserId}/test-webhook — inline {message}（bot_agent.go test-webhook）。
bot_dyn_cmd!(
    BotAgentTestWebhookCommand,
    BOT_AGENT_TEST_WEBHOOK_REG,
    "im_bot_agent_test_webhook",
    "bot-agent/config",
    |args, cmd| Ok(format!(
        "bot-agent/config/{}/test-webhook",
        path_seg(args, "bot_user_id", cmd)?
    )),
    |args, cmd| Ok(json!({ "message": require_str(args, "message", cmd)? }))
);

// #18 GET /bot-agent/channel/{channelId}/bots — path param。
bot_dyn_cmd!(
    read BotAgentChannelBotsCommand,
    BOT_AGENT_CHANNEL_BOTS_REG,
    "im_bot_agent_channel_bots",
    "bot-agent/channel",
    |args, cmd| Ok(format!(
        "bot-agent/channel/{}/bots",
        path_seg(args, "channel_id", cmd)?
    )),
    |_args, _cmd| Ok(json!({}))
);

// #19 GET /bot-agent/channel/{channelId}/available-bots — path param。
bot_dyn_cmd!(
    read BotAgentAvailableBotsCommand,
    BOT_AGENT_AVAILABLE_BOTS_REG,
    "im_bot_agent_channel_available_bots",
    "bot-agent/channel",
    |args, cmd| Ok(format!(
        "bot-agent/channel/{}/available-bots",
        path_seg(args, "channel_id", cmd)?
    )),
    |_args, _cmd| Ok(json!({}))
);

// #20 POST /bot-agent/channel/{channelId}/bot/{botUserId} — addedBy=session.UserId（服务端），双 path param。
bot_dyn_cmd!(
    BotAgentAddToChannelCommand,
    BOT_AGENT_ADD_TO_CHANNEL_REG,
    "im_bot_agent_channel_add",
    "bot-agent/channel",
    |args, cmd| Ok(format!(
        "bot-agent/channel/{}/bot/{}",
        path_seg(args, "channel_id", cmd)?,
        path_seg(args, "bot_user_id", cmd)?
    )),
    |_args, _cmd| Ok(json!({}))
);

// #21 DELETE /bot-agent/channel/{channelId}/bot/{botUserId} — 双 path param。
bot_dyn_cmd!(
    BotAgentRemoveFromChannelCommand,
    BOT_AGENT_REMOVE_FROM_CHANNEL_REG,
    "im_bot_agent_channel_remove",
    "bot-agent/channel",
    |args, cmd| Ok(format!(
        "bot-agent/channel/{}/bot/{}",
        path_seg(args, "channel_id", cmd)?,
        path_seg(args, "bot_user_id", cmd)?
    )),
    |_args, _cmd| Ok(json!({}))
);
