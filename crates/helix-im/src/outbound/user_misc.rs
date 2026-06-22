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
//! - notification/* = `api/cses/notification`（api.go:181）：`/loadSend`（notification.go:13）。
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

use super::registry::{require_str, OutboundCommand, OutboundRegistration, OUTBOUND_COMMANDS};

// ── 本文件局部 helper（边界零信任：缺/类型错 → Err，不 panic）──────────────────────

/// 取必填字符串数组（非空、元素全 string）。供 `{userIds:[]}` 类命令复用。
fn require_str_array(args: &Value, key: &str, cmd: &str) -> Result<Value, ImError> {
    args.get(key)
        .and_then(Value::as_array)
        .filter(|a| !a.is_empty() && a.iter().all(Value::is_string))
        .cloned()
        .map(Value::Array)
        .ok_or_else(|| ImError::Parse(format!("{cmd}: 缺/空 {key}（非空字符串数组）")))
}

/// 注册一条命令的样板：`name` + `build` 闭包（与 posts_read.rs `read_cmd!` 同构，独立宏不跨文件耦合）。
macro_rules! misc_cmd {
    ($cmd_struct:ident, $reg:ident, $name:literal, $build:expr) => {
        struct $cmd_struct;
        impl OutboundCommand for $cmd_struct {
            fn name(&self) -> &'static str {
                $name
            }
            fn build(&self, args: &Value) -> Result<(&'static str, Value), ImError> {
                let f: fn(&Value, &'static str) -> Result<(&'static str, Value), ImError> = $build;
                f(args, $name)
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
    |args, cmd| {
        let user_ids = require_str_array(args, "user_ids", cmd)?;
        Ok(("users/status/ids", json!({ "userIds": user_ids })))
    }
);

// ── teams/*（personal user → 403 PERSONAL_USER_FORBIDDEN，响应态，outbound 不裁决）─────────

// #3 teams/upsert：维护公司大群（非 team 表）。真源 r.Body 原样透传（team.go:63 `UpsertTeam(session, r.Body)`）。
// 现网前端传整个 team 对象 body；helix 直透传 `team`（不拆字段，由前端给完整 wire 对象）。
misc_cmd!(
    TeamUpsertCommand,
    TEAM_UPSERT_REG,
    "im_team_upsert",
    |args, cmd| {
        let team = args
            .get("team")
            .filter(|v| v.is_object())
            .cloned()
            .ok_or_else(|| ImError::Parse(format!("{cmd}: 缺/坏 team（对象）")))?;
        Ok(("teams/upsert", team))
    }
);

// #4 teams/member/add：变更团队成员。真源 ChannelMemberEvent（team.go:42 decode）。整 event 对象透传。
misc_cmd!(
    TeamMemberAddCommand,
    TEAM_MEMBER_ADD_REG,
    "im_team_member_add",
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
// 真源 QuitTeamReq{userId,teamId}（quit_team.go:3-6）。**注**：现网 Methods("DELETE")，body 仍带；
// helix outbound 当前 registry 恒发 POST（http_post 单点）——method 差异属 driver 横切层关切，
// P5a build 层只保证 endpoint+body 对齐；DELETE 语义 driver 接缝（P6）按命令名映射时校正。
misc_cmd!(
    TeamQuitCommand,
    TEAM_QUIT_REG,
    "im_team_quit",
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

// ── search 四件套（body = DoSearchRequest，camelCase tag）────────────────────────────
//
// DoSearchRequest（post_search_param.go:25-34）：keyword?/searchType?(int)/postType?/channelType?/
// channelId/teamId?/isAll/userId(`json:"-"` 服务端从 session 注入，前端**不传**)。
// search/do = searchAll → 响应含 channels/users/**channelDetails** 第四 key（cses_search.go:42-54），
// 属响应态；P5a build 层只产请求体。

/// 组装 DoSearchRequest wire body（仅下发显式给出的字段，全 camelCase；userId 由服务端注入不传）。
/// channelId 必填（DoSearchRequest.ChannelId 非指针），其余可选。
fn do_search_body(args: &Value, cmd: &str) -> Result<Value, ImError> {
    let channel_id = args.get("channel_id").and_then(Value::as_str).unwrap_or("");
    let mut b = json!({ "channelId": channel_id });
    if let Some(keyword) = args.get("keyword").and_then(Value::as_str) {
        b["keyword"] = json!(keyword);
    }
    if let Some(search_type) = args.get("search_type").and_then(Value::as_i64) {
        b["searchType"] = json!(search_type);
    }
    if let Some(team_id) = args.get("team_id").and_then(Value::as_str) {
        b["teamId"] = json!(team_id);
    }
    if let Some(is_all) = args.get("is_all").and_then(Value::as_bool) {
        b["isAll"] = json!(is_all);
    }
    // userId 是 `json:"-"`（服务端 session 注入）；前端传了也禁止下发（防契约漂移）。
    if args.get("user_id").is_some() {
        return Err(ImError::Parse(format!(
            "{cmd}: user_id 由服务端 session 注入，禁前端下发（DoSearchRequest.UserId json:\"-\"）"
        )));
    }
    Ok(b)
}

// #8 search/post：消息搜索。
misc_cmd!(
    SearchPostCommand,
    SEARCH_POST_REG,
    "im_search_post",
    |args, cmd| { Ok(("search/post", do_search_body(args, cmd)?)) }
);

// #9 search/user：用户搜索。
misc_cmd!(
    SearchUserCommand,
    SEARCH_USER_REG,
    "im_search_user",
    |args, cmd| { Ok(("search/user", do_search_body(args, cmd)?)) }
);

// #10 search/channel：频道搜索。
misc_cmd!(
    SearchChannelCommand,
    SEARCH_CHANNEL_REG,
    "im_search_channel",
    |args, cmd| { Ok(("search/channel", do_search_body(args, cmd)?)) }
);

// #11 search/do：聚合搜索（searchAll → channels/users/channelDetails 四 key 响应）。
misc_cmd!(
    SearchDoCommand,
    SEARCH_DO_REG,
    "im_search_do",
    |args, cmd| { Ok(("search/do", do_search_body(args, cmd)?)) }
);
