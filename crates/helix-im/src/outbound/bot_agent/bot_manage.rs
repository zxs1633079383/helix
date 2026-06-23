//! Bot 管理 CRUD outbound（#3-11，session 鉴权，bot-manage/* 子路由 bot_manage.go:14-22）。
//!
//! 含 query-param GET（#6 token/list `?botUserId=` bot_manage.go:76 / #11 visible-user/list
//! bot_manage.go:157）——经 `bot_dyn_cmd!` 的 `path_override` 把 botUserId 插进 query string；
//! method（GET）差异属 driver 接缝 P6（与 P5a teams/quit DELETE 同款，registry 恒 POST）。

use serde_json::{json, Value};

use super::path_seg;
use crate::error::ImError;
use crate::outbound::registry::require_str;

// #3 GET /bot-manage/list — 从 session 取 userId/companyId，无请求体。
bot_cmd!(
    read ListVisibleBotsCommand,
    LIST_VISIBLE_BOTS_REG,
    "im_bot_list_visible",
    |_args, _cmd| { Ok(("bot-manage/list", json!({}))) }
);

// #4 POST /bot-manage/create — CreateBotRequest{name,description,teamId?}（bot_manage.go）。
bot_cmd!(
    CreateBotManageCommand,
    CREATE_BOT_MANAGE_REG,
    "im_bot_create",
    |args, cmd| {
        let name = require_str(args, "name", cmd)?;
        let description = args
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or("");
        let mut b = json!({ "name": name, "description": description });
        if let Some(team_id) = args.get("team_id").and_then(Value::as_str) {
            b["teamId"] = json!(team_id);
        }
        Ok(("bot-manage/create", b))
    }
);

// #5 POST /bot-manage/token/generate — GenerateTokenRequest{botUserId,description,expireAt?}.
bot_cmd!(
    GenerateTokenCommand,
    GENERATE_TOKEN_REG,
    "im_bot_token_generate",
    |args, cmd| {
        let bot_user_id = require_str(args, "bot_user_id", cmd)?;
        let description = args
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or("");
        let mut b = json!({ "botUserId": bot_user_id, "description": description });
        if let Some(expire_at) = args.get("expire_at").and_then(Value::as_i64) {
            b["expireAt"] = json!(expire_at);
        }
        Ok(("bot-manage/token/generate", b))
    }
);

// #6 GET /bot-manage/token/list?botUserId= — query param 必填（bot_manage.go:76）。
bot_dyn_cmd!(
    read ListTokensCommand,
    LIST_TOKENS_REG,
    "im_bot_token_list",
    "bot-manage/token/list",
    |args, cmd| Ok(format!(
        "bot-manage/token/list?botUserId={}",
        path_seg(args, "bot_user_id", cmd)?
    )),
    |_args, _cmd| Ok(json!({}))
);

// #7 POST /bot-manage/token/revoke — RevokeTokenRequest{tokenId}.
bot_cmd!(
    RevokeTokenCommand,
    REVOKE_TOKEN_REG,
    "im_bot_token_revoke",
    |args, cmd| {
        let token_id = require_str(args, "token_id", cmd)?;
        Ok(("bot-manage/token/revoke", json!({ "tokenId": token_id })))
    }
);

// #8 POST /bot-manage/scope/set — SetScopeRequest{botUserId,scopeType,teamId?}.
bot_cmd!(
    SetScopeCommand,
    SET_SCOPE_REG,
    "im_bot_scope_set",
    |args, cmd| {
        let bot_user_id = require_str(args, "bot_user_id", cmd)?;
        let scope_type = require_str(args, "scope_type", cmd)?;
        let mut b = json!({ "botUserId": bot_user_id, "scopeType": scope_type });
        if let Some(team_id) = args.get("team_id").and_then(Value::as_str) {
            b["teamId"] = json!(team_id);
        }
        Ok(("bot-manage/scope/set", b))
    }
);

// #9 POST /bot-manage/scope/visible-user/add — VisibleUserRequest{botUserId,userId}.
bot_cmd!(
    AddVisibleUserCommand,
    ADD_VISIBLE_USER_REG,
    "im_bot_visible_user_add",
    |args, cmd| {
        Ok((
            "bot-manage/scope/visible-user/add",
            visible_user_body(args, cmd)?,
        ))
    }
);

// #10 POST /bot-manage/scope/visible-user/remove — VisibleUserRequest{botUserId,userId}.
bot_cmd!(
    RemoveVisibleUserCommand,
    REMOVE_VISIBLE_USER_REG,
    "im_bot_visible_user_remove",
    |args, cmd| {
        Ok((
            "bot-manage/scope/visible-user/remove",
            visible_user_body(args, cmd)?,
        ))
    }
);

// #11 GET /bot-manage/scope/visible-user/list?botUserId= — query param 必填（bot_manage.go:157）。
bot_dyn_cmd!(
    read ListVisibleUsersCommand,
    LIST_VISIBLE_USERS_REG,
    "im_bot_visible_user_list",
    "bot-manage/scope/visible-user/list",
    |args, cmd| Ok(format!(
        "bot-manage/scope/visible-user/list?botUserId={}",
        path_seg(args, "bot_user_id", cmd)?
    )),
    |_args, _cmd| Ok(json!({}))
);

/// VisibleUserRequest{botUserId,userId}（#9/#10 共用）。
fn visible_user_body(args: &Value, cmd: &str) -> Result<Value, ImError> {
    let bot_user_id = require_str(args, "bot_user_id", cmd)?;
    let user_id = require_str(args, "user_id", cmd)?;
    Ok(json!({ "botUserId": bot_user_id, "userId": user_id }))
}
