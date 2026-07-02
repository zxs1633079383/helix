//! user-misc 域 outbound 命令（P5a，parallelGroup=G2）。
//!
//! 纯**读/写** request：helix-im 只 build 正确 wire body，driver fire HTTP（鉴权头/401/断网/trace
//! 由 P1.5 横切层注入，core 不碰 token，HX-C001/C6）。与 P5b `bot_agent.rs` 真零交集：独立文件 +
//! distributed_slice 注册，不碰 registry.rs/channel*/posts_read.rs（仅 mod.rs append 一行）。
//!
//! ## endpoint / body 真源（mattermost csesapi 逐字 json tag，不自造 fixture，C5）
//! - users/* = `api/cses/users`（api.go:173）：`/list`（user.go:15）、`/status/ids`（user.go:16）。
//! - teams/* = `api/cses/teams`（api.go:174）：`/upsert`、`/member/add`、`/member/quit`（team.go:13-15）。
//! - modules/* = `api/cses/modules`（api.go:175）：`/getAll`（modules.go:11）。
//! - notification/* = `api/cses/notification`（api.go:181）：`/loadSend`（notification.go:13）、
//!   `/loadTarget`（notification.go:14，姊妹路由，req=query.Page 无 state）。
//! - search/* = `api/cses/search`（api.go:182）：`/post`、`/user`、`/channel`、`/do`（search.go:13-16）。
//!
//! ## presence（D2 已拍：helix 自建）
//! `im_get_user_statuses` = `users/status/ids` 批量在线状态接管（user.go:41 `{userIds:[]string}`）——
//! 接管完整性前置（presence/头像是渲染必需，不接前端走老通道拿 presence = 纯渲染破功）。
//! WS 连接态聚合（自建 presence 的推送态）属后续能力；P5a 落地的是 presence 拉取 outbound 接管。
//!
//! ## 源 bug 移植纪律（接管 = 透传现网行为，非修源）
//! 现网 4 处源 bug（notification loadSend↔loadTarget handler 互换 / quitTeam(DELETE) 无响应体 /
//! Page offset=Page*Size / modules getAllModules `SetMessage("channelMemberChange success")` 笔误）
//! 均属**服务端响应/handler 行为**态——helix outbound 只 build **请求体**，不解析响应、不绕过、不预修。
//! P5a 断言 outbound endpoint+body 与现网 decode 结构一致即可（响应态 bug 不在 build 层裁决）。
//! `PERSONAL_USER_FORBIDDEN`（personal_guard.go:13 verbatim）是 teams/* 的 403 响应码，亦响应态。

use linkme::distributed_slice;
use serde_json::{json, Value};

use crate::error::ImError;

use crate::outbound::registry::{
    build_creator_member_users, require_str, OutboundCommand, OutboundRegistration,
    OUTBOUND_COMMANDS,
};
// build 层 helper 外提 sibling（收口行数基线，纯函数零行为变更）。
use super::helpers::{do_search_body, require_str_array};

/// 注册一条命令的样板：`name` + `build` 闭包 + **读/写标注**（user-misc 域读写混存，
/// 与纯读 posts_read.rs 不同，须逐命令标 `is_read`，spec06 缺陷A）。
///
/// 末参 `read` = `true`（读族：无 WS 回声，HTTP 响应即数据 → dispatch 注册 OutboundReadReply 回灌）/
/// `false`（写族：数据走 WS 回声，HTTP 响应可丢 fire-and-forget）。
macro_rules! misc_cmd {
    ($cmd_struct:ident, $reg:ident, $name:literal, $is_read:literal, $build:expr) => {
        struct $cmd_struct;
        impl OutboundCommand for $cmd_struct {
            fn name(&self) -> &'static str {
                $name
            }
            fn build(&self, args: &Value) -> Result<(&'static str, Value), ImError> {
                let f: fn(&Value, &'static str) -> Result<(&'static str, Value), ImError> = $build;
                f(args, $name)
            }
            fn is_read(&self) -> bool {
                $is_read
            }
        }
        // 单元 struct 值经 const promotion 直接做 &'static dyn 注册目标（无需中间 static）。
        #[distributed_slice(OUTBOUND_COMMANDS)]
        static $reg: OutboundRegistration = OutboundRegistration {
            name: $name,
            command: &$cmd_struct,
        };
    };
}

// ── users/* ──────────────────────────────────────────────────────────────────────

// #1 users/list：按 channel 拉成员用户。真源 `{channelId}`（user.go:23 `ChannelId string json:"channelId"`）。
misc_cmd!(
    ListUsersCommand,
    LIST_USERS_REG,
    "im_list_users",
    true,
    |args, cmd| {
        let channel_id = require_str(args, "channel_id", cmd)?;
        Ok(("users/list", json!({ "channelId": channel_id })))
    }
);

// #2 users/status/ids：批量在线状态（presence 接管，D2）。真源 `{userIds:[]string}`（user.go:43）。
misc_cmd!(
    GetUserStatusesCommand,
    GET_USER_STATUSES_REG,
    "im_get_user_statuses",
    true,
    |args, cmd| {
        let user_ids = require_str_array(args, "user_ids", cmd)?;
        Ok(("users/status/ids", json!({ "userIds": user_ids })))
    }
);

// ── teams/*（personal user → 403 PERSONAL_USER_FORBIDDEN，响应态，outbound 不裁决）─────────

// #3 teams/upsert：维护公司大群（非 team 表）。真源 r.Body = CreateChannelSpecifyOwner（team.go:63
// `UpsertTeam(session, r.Body)`，wire body 即 team 对象，无外层包裹）。
//
// **body 成形下沉 helix（薄壳合规）**：壳只传结构化 args `{display_name, team_id, self_id,
// member_ids:[]}`；team 对象 + users[] + owner(CREATOR) 业务赋值在此拼成，对齐真机curl真源 partials/3
// §4：`{teamId, displayName, orient:"", type:"P", picturetype:"USER", picture:{userIds:[...]},
// users:[{id,teamId,role}], forceCreate:true, owner:{id,teamId,role:"CREATOR"}}`。不携 `id` →
// server 走建群分支（CreateCsesChannel·触 channel_created）。
misc_cmd!(
    TeamUpsertCommand,
    TEAM_UPSERT_REG,
    "im_team_upsert",
    false,
    |args, cmd| {
        let display_name = require_str(args, "display_name", cmd)?;
        let self_id = require_str(args, "self_id", cmd)?;
        let team_id = args.get("team_id").and_then(Value::as_str).unwrap_or("");
        let (users, user_ids) = build_creator_member_users(args, cmd)?;
        let team = json!({
            "teamId": team_id,
            "displayName": display_name,
            "orient": "",
            "type": "P",
            "picturetype": "USER",
            "picture": { "userIds": user_ids },
            "users": users,
            "forceCreate": true,
            "owner": { "id": self_id, "teamId": team_id, "role": "CREATOR" },
        });
        Ok(("teams/upsert", team))
    }
);

// #4 teams/member/add：变更团队成员。真源 ChannelMemberEvent（team.go:42 decode）。整 event 对象透传。
misc_cmd!(
    TeamMemberAddCommand,
    TEAM_MEMBER_ADD_REG,
    "im_team_member_add",
    false,
    |args, cmd| {
        let event = args
            .get("event")
            .filter(|v| v.is_object())
            .cloned()
            .ok_or_else(|| {
                ImError::Parse(format!("{cmd}: 缺/坏 event（ChannelMemberEvent 对象）"))
            })?;
        Ok(("teams/member/add", event))
    }
);

// #5 teams/member/quit：退出团队（HTTP DELETE，无响应体 = 现网源 bug，透传）。
// 真源 QuitTeamReq{userId,teamId}（quit_team.go:3-6）。现网 Methods("DELETE") 且 body 仍带；
// Task1 起 registry 按命令名产 DELETE，build 层仍只负责 endpoint+body。
misc_cmd!(
    TeamQuitCommand,
    TEAM_QUIT_REG,
    "im_team_quit",
    false,
    |args, cmd| {
        let user_id = require_str(args, "user_id", cmd)?;
        let team_id = require_str(args, "team_id", cmd)?;
        Ok((
            "teams/member/quit",
            json!({ "userId": user_id, "teamId": team_id }),
        ))
    }
);

// ── modules/* ─────────────────────────────────────────────────────────────────────

// #6 modules/getAll：拉会话分组模块。真源**空 body** POST（modules.go:14 不 decode 请求体；
// 前端 message.service.ts:828 `post$('/modules/getAll')` 无 body）。
misc_cmd!(
    GetAllModulesCommand,
    GET_ALL_MODULES_REG,
    "im_get_all_modules",
    true,
    |_args, _cmd| { Ok(("modules/getAll", json!({}))) }
);

// ── notification/*（loadSend↔loadTarget handler 互换 = 现网源 bug，透传不修）──────────────

// #7 notification/loadSend：拉通知（NotificationReq 嵌 Page{page,size} + state）。
// 真源 query.NotificationReq{Page{page,size}, state}（NotificationReq.go:3-6 + page.go:3-6）。
// page/size validate required,min=1（前端须给 ≥1）；缺则下发 0（Go 零值，校验在服务端）。
misc_cmd!(
    LoadSendNotificationsCommand,
    LOAD_SEND_NOTIFICATIONS_REG,
    "im_load_send_notifications",
    true,
    |args, cmd| {
        // page/size 必填（Page validate required,min=1）；state 可选。
        let page = args
            .get("page")
            .and_then(Value::as_i64)
            .ok_or_else(|| ImError::Parse(format!("{cmd}: 缺/坏 page（int ≥1）")))?;
        let size = args
            .get("size")
            .and_then(Value::as_i64)
            .ok_or_else(|| ImError::Parse(format!("{cmd}: 缺/坏 size（int ≥1）")))?;
        let mut b = json!({ "page": page, "size": size });
        if let Some(state) = args.get("state").and_then(Value::as_str) {
            b["state"] = json!(state);
        }
        Ok(("notification/loadSend", b))
    }
);

// #7b notification/loadTarget：拉接收侧通知（与 loadSend 同域的姊妹路由，真路由真 handler）。
// 真源 query.Page（**handler 默认初始化 `{Page:0, Size:10}`**，notification.go:40-41）——与 loadSend
// 用 NotificationReq{Page,state} 不同：loadTarget 仅 `{page,size}`，**无 state 字段**。
// 缺省即取 Go handler 同款默认（page=0/size=10），与服务端 pre-init 对齐（前端可覆写）。
// 现网命名混淆（handler loadReceive ↔ app LoadSend）+ store 返回 nil 属**响应态**——outbound 只
// build 请求体不裁决（src bug 移植纪律，见文件头）。读族（无 WS 回声，HTTP 响应即数据）。
misc_cmd!(
    LoadTargetNotificationsCommand,
    LOAD_TARGET_NOTIFICATIONS_REG,
    "im_load_target_notifications",
    true,
    |args, _cmd| {
        // query.Page 默认 {Page:0, Size:10}（notification.go:40）；前端可覆写。无 state 字段。
        let page = args.get("page").and_then(Value::as_i64).unwrap_or(0);
        let size = args.get("size").and_then(Value::as_i64).unwrap_or(10);
        Ok((
            "notification/loadTarget",
            json!({ "page": page, "size": size }),
        ))
    }
);

// ── search 四件套（body = DoSearchRequest，camelCase tag）────────────────────────────
//
// DoSearchRequest（post_search_param.go:25-34）：keyword?/searchType?(int)/postType?/channelType?/
// channelId/teamId?/isAll/userId(`json:"-"` 服务端从 session 注入，前端**不传**)。
// search/do = searchAll → 响应含 channels/users/**channelDetails** 第四 key（cses_search.go:42-54），
// 属响应态；P5a build 层只产请求体。`do_search_body` 组装逻辑外提 user_misc_helpers（收口行数基线）。

// #8 search/post：消息搜索。
misc_cmd!(
    SearchPostCommand,
    SEARCH_POST_REG,
    "im_search_post",
    true,
    |args, cmd| { Ok(("search/post", do_search_body(args, cmd)?)) }
);

// #9 search/user：用户搜索。
misc_cmd!(
    SearchUserCommand,
    SEARCH_USER_REG,
    "im_search_user",
    true,
    |args, cmd| { Ok(("search/user", do_search_body(args, cmd)?)) }
);

// #10 search/channel：频道搜索。
misc_cmd!(
    SearchChannelCommand,
    SEARCH_CHANNEL_REG,
    "im_search_channel",
    true,
    |args, cmd| { Ok(("search/channel", do_search_body(args, cmd)?)) }
);

// #11 search/do：聚合搜索（searchAll → channels/users/channelDetails 四 key 响应）。
// spec06 §4：search_do = Go 未实现（Java 全局搜索遗留），唯一合法「暂不切」——前端走 legacy，
// helix 不接读回灌（标 `false`，不注册 OutboundReadReply）。endpoint+body 仍保真（待 Go 补齐后切）。
misc_cmd!(
    SearchDoCommand,
    SEARCH_DO_REG,
    "im_search_do",
    false,
    |args, cmd| { Ok(("search/do", do_search_body(args, cmd)?)) }
);

#[cfg(test)]
#[path = "core_tests.rs"]
mod tests;
