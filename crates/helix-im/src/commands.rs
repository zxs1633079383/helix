//! Outbound 应用命令 → `Effect::Http`（Tier 1 自驱动遥控器的业务侧）。
//!
//! `im_send_message` 因带乐观落库 / timer / PendingSend 对账，仍留 `module.rs`。
//! 本文件收口**纯 HTTP-fire** 的 outbound 命令：解析通用 snake_case 入参 → 翻成现网
//! camelCase Go body → 一条 `Effect::Http` POST 到现网真源 endpoint。
//!
//! **endpoint/body 真源**：`docs/csesapi-migration/full-map/partials/{1,2}-*.md`（逐条标注）。
//! 业务契约（ubiquitous language）归 helix-im（Bounded Context），host/repl 只供用户参数。
//! 身份头：`connectionId`（im 收 hello 盖章）经 [`to_effect::connection_id_headers`] 单点收口；
//! `cookieId`(=userId) 是 driver 全局头（host 注入），不在此（core 不认识 userId）。

use bytes::Bytes;
use helix_core::effect::{Effect, HttpRequest};
use helix_core::Correlation;
use serde_json::{json, Value};

use crate::error::ImError;

/// 本模块认领的 outbound 命令名（`accepts_tick` + `module.rs` dispatch 共用单一真源）。
pub fn is_outbound(name: &str) -> bool {
    matches!(
        name,
        "im_post_read"
            | "im_revoke"
            | "im_channel_leave"
            | "im_create_schedule"
            | "im_cancel_schedule"
            | "im_create_channel"
            | "im_make_topic"
            | "im_channel_member_change"
            | "im_channel_close"
            | "im_update_member_nickname"
    )
}

/// 把一条 outbound 命令翻成 Effect 列表（当前恒为单条 `Effect::Http`）。
///
/// 入参 `payload` = repl/前端供的 JSON（snake_case 通用参数）；本函数据 `name` 选
/// 现网 endpoint + 组装 Go body。解析失败 → `ImError::Parse`（边界零信任，HX-C 不变量 4）。
pub fn handle_outbound(
    name: &str,
    payload: &[u8],
    api_base_url: &str,
    connection_id: Option<&str>,
    corr: Correlation,
) -> Result<Vec<Effect>, ImError> {
    let args: Value = serde_json::from_slice(payload)
        .map_err(|e| ImError::Parse(format!("{name} payload: {e}")))?;

    let (path, body) = match name {
        // #23 POST /api/cses/post/read — 标记已读 → update_channel + post_read
        // 真源 entity.PostRead{ChannelID, EndTime, ...}；UserId nil → Go 用 session 兜底。
        "im_post_read" => {
            let channel_id = require_str(&args, "channel_id", name)?;
            let mut b = json!({ "channelId": channel_id });
            if let Some(end_time) = args.get("end_time").and_then(Value::as_i64) {
                b["endTime"] = json!(end_time);
            }
            ("post/read", b)
        }
        // #14 POST /api/cses/posts/revoke — 撤回 → post_update。真源 {PostId}。
        "im_revoke" => {
            let post_id = require_str(&args, "post_id", name)?;
            ("posts/revoke", json!({ "postId": post_id }))
        }
        // #33 POST /api/cses/channel/member/leave — 退群（仅自己）→ channel_member_update + post。
        // 真源 entity.ChannelIdStruct{ChannelId}；UserId nil → session（Go 只允许自己退）。
        "im_channel_leave" => {
            let channel_id = require_str(&args, "channel_id", name)?;
            ("channel/member/leave", json!({ "channelId": channel_id }))
        }
        // #3 POST /api/cses/posts/createSchedule — 定时消息 → post_schedule_created。
        // 真源 CreateSchedulePostReq{Post *Post, ScheduledTime int64 `json:"schedulePostAt"`}。
        "im_create_schedule" => {
            let channel_id = require_str(&args, "channel_id", name)?;
            let message = require_str(&args, "message", name)?;
            let schedule_post_at = args
                .get("schedule_post_at")
                .and_then(Value::as_i64)
                .ok_or_else(|| {
                    ImError::Parse(format!("{name}: 缺/坏 schedule_post_at（int64 毫秒）"))
                })?;
            let mut post = json!({ "channelId": channel_id, "message": message });
            if let Some(tmp) = args.get("temporary_id").and_then(Value::as_str) {
                post["temporaryId"] = json!(tmp);
            }
            (
                "posts/createSchedule",
                json!({ "post": post, "schedulePostAt": schedule_post_at }),
            )
        }
        // #4 POST /api/cses/posts/cancelSchedule — 取消定时 → post_schedule_canceled。
        // 真源 CancelSchedulePostReq{ChannelId}；userId 取 session。
        "im_cancel_schedule" => {
            let channel_id = require_str(&args, "channel_id", name)?;
            ("posts/cancelSchedule", json!({ "channelId": channel_id }))
        }
        // #7 POST /api/cses/channel/create — 建群 → channel_created + channel_member_update + post。
        // 真源 CreateChannelSpecifyOwner（内嵌 *ent.Channel + Users[]ChannelMemberMinimal）字段多，
        // 由 repl 直供 raw body（args 即 Go body 直传，helix-im 只认领 endpoint）。
        "im_create_channel" => ("channel/create", args.clone()),
        // #17 POST /api/cses/posts/makeTopic — 消息转话题 → channel_created + post_update。
        // 真源 {PostId, 内嵌 CreateChannelSpecifyOwner}；repl 直供 raw body（须含 postId）。
        "im_make_topic" => {
            // 至少校验 postId 存在（边界零信任），其余字段直传。
            require_str(&args, "postId", name)?;
            ("posts/makeTopic", args.clone())
        }
        // POST /api/cses/channel/member/change — 加/退成员 → channel_member_update。
        // 真源 §5（真机curl真源）：{channelId, joinUsers?:[{id,teamId,role}], leaveUsers?:[...]}，
        // joinUsers/leaveUsers 两者可同时非空。成员对象结构（ChannelMemberMinimal）较复杂，由
        // repl 直供 raw body（args 即 Go body 直传）；至少校验 channelId 存在（边界零信任）。
        // 注意：与 #33 `channel/member/leave`（自己退群）是两条——本条是 admin 增减成员。
        "im_channel_member_change" => {
            require_str(&args, "channelId", name)?;
            ("channel/member/change", args.clone())
        }
        // #27 POST /api/cses/channel/close — 关闭频道 → channel_close（broadcast 到 channelId·自己也收）。真源 {ChannelId}。
        "im_channel_close" => {
            let channel_id = require_str(&args, "channel_id", name)?;
            ("channel/close", json!({ "channelId": channel_id }))
        }
        // #31 POST /api/cses/channel/member/change/nickname — 改群昵称 → update_channel_member_nickName
        // （注意现网 WS 名 camelCase `nickName`，broadcast 到 channelId）。真源 {ChannelId, UserId(nil→session), Nickname(trim 空→清空)}。
        "im_update_member_nickname" => {
            let channel_id = require_str(&args, "channel_id", name)?;
            // nickname 允许空（trim 空→Go 侧清空昵称），故不 require 非空。
            let nickname = args.get("nickname").and_then(Value::as_str).unwrap_or("");
            let mut b = json!({ "channelId": channel_id, "nickname": nickname });
            if let Some(uid) = args.get("user_id").and_then(Value::as_str) {
                b["userId"] = json!(uid);
            }
            ("channel/member/change/nickname", b)
        }
        other => return Err(ImError::Parse(format!("未认领的 outbound 命令 '{other}'"))),
    };

    Ok(vec![http_post(
        api_base_url,
        path,
        body,
        connection_id,
        corr,
    )])
}

/// 组装一条 `Effect::Http` POST（Content-Type + connectionId 身份头）。
fn http_post(
    api_base_url: &str,
    path: &str,
    body: Value,
    connection_id: Option<&str>,
    corr: Correlation,
) -> Effect {
    let mut headers = vec![("Content-Type".to_string(), "application/json".to_string())];
    headers.extend(crate::acl::to_effect::connection_id_headers(connection_id));
    Effect::Http {
        corr,
        req: HttpRequest {
            method: "POST".to_string(),
            url: format!("{api_base_url}/{path}"),
            headers,
            body: Some(Bytes::from(serde_json::to_vec(&body).unwrap_or_default())),
        },
    }
}

/// 取必填字符串字段，缺/类型错 → `ImError::Parse`（边界零信任）。
fn require_str<'a>(args: &'a Value, key: &str, cmd: &str) -> Result<&'a str, ImError> {
    args.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ImError::Parse(format!("{cmd}: 缺/空必填字段 '{key}'")))
}
