//! posts 读族 outbound 命令（P3a，parallelGroup=G1）。
//!
//! 纯**读** request/response：helix-im 只 build 正确 wire body。读族无 WS 回声、HTTP 响应体即数据
//! → `is_read=true`（spec06 缺陷A），dispatch 注册 `OutboundReadReply` 把响应透传回灌前端（read_relay）。
//! 与 P3b 真零交集：独立文件 + distributed_slice 注册，不碰 channel*/registry.rs（仅 mod.rs append）。
//! ## endpoint / body 真源（mattermost csesapi 逐字，不自造 fixture，C5）
//! - posts/* 子路由 = `api/cses/posts`（api.go:169）；post/* = `api/cses/post`（api.go:171）；
//!   post/bookmark/* = `api/cses/post/bookmark`（api.go:177）。
//! - 命名陷阱（decode 静默失败防线）：top20 → **snake_case** `channel_id`（posts.go:550）；
//!   getPostsAfterIndex → `postIds` **单数 string**（posts.go:318，非数组）；
//!   getUpdatedPosts → `timeStamp` **大写 S**（posts.go:体 CursorTs tag）；时间 `createAt`。

use linkme::distributed_slice;
use serde_json::{json, Value};

use crate::error::ImError;

use super::registry::{require_str, OutboundCommand, OutboundRegistration, OUTBOUND_COMMANDS};

// ── 本文件局部 helper（边界零信任：缺/类型错 → Err，不 panic）──────────────────────

/// 取必填字符串数组（非空、元素全 string）。供 `{postIds:[]}` 类命令复用。
fn require_str_array(args: &Value, key: &str, cmd: &str) -> Result<Value, ImError> {
    args.get(key)
        .and_then(Value::as_array)
        .filter(|a| !a.is_empty() && a.iter().all(Value::is_string))
        .cloned()
        .map(Value::Array)
        .ok_or_else(|| ImError::Parse(format!("{cmd}: 缺/空 {key}（非空字符串数组）")))
}

/// 注册读命令样板：`name` + `build` 闭包 → struct/static/slice 三件套（`is_read=true` 全读族）。
macro_rules! read_cmd {
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
            // 全读族（spec06 缺陷A）：无 WS 回声、HTTP 响应体即数据 → dispatch 注册回灌。
            fn is_read(&self) -> bool {
                true
            }
        }
        // 单元 struct 值（`$cmd_struct` 同名构造子）经 const promotion 直接做 &'static dyn 注册
        // 目标——无需中间 static（避免与类型同名在 value 命名空间冲突 E0428）。
        #[distributed_slice(OUTBOUND_COMMANDS)]
        static $reg: OutboundRegistration = OutboundRegistration {
            name: $name,
            command: &$cmd_struct,
        };
    };
}

// ── posts/* 读族 ────────────────────────────────────────────────────────────────

// #1 getSchedule：查询定时消息。真源 GetSchedulePostReq{channelId,userId}（schedule_post.go:22 必填 userId）。
read_cmd!(
    GetScheduleCommand,
    GET_SCHEDULE_REG,
    "im_get_schedule",
    |args, cmd| {
        let channel_id = require_str(args, "channel_id", cmd)?;
        let user_id = require_str(args, "user_id", cmd)?;
        Ok((
            "posts/getSchedule",
            json!({ "channelId": channel_id, "userId": user_id }),
        ))
    }
);

// #2 postContext：取某 post 上下文（上拉锚点）。真源 {postId, before:int}（posts.go:564）。
read_cmd!(
    PostContextCommand,
    POST_CONTEXT_REG,
    "im_post_context",
    |args, cmd| {
        let post_id = require_str(args, "post_id", cmd)?;
        let before = args.get("before").and_then(Value::as_i64).unwrap_or(0);
        Ok((
            "posts/postContext",
            json!({ "postId": post_id, "before": before }),
        ))
    }
);

// #3 top20：首屏 20 条。**命名陷阱**：body 是 snake_case `channel_id`（posts.go:550 唯一 snake 读端）。
read_cmd!(Top20Command, TOP20_REG, "im_top20", |args, cmd| {
    let channel_id = require_str(args, "channel_id", cmd)?;
    Ok(("posts/top20", json!({ "channel_id": channel_id })))
});

// #4 get：批量按 id 取 post。真源 {postIds:*[]string}（posts.go:535，复数数组）。
read_cmd!(
    GetPostsCommand,
    GET_POSTS_REG,
    "im_get_posts",
    |args, cmd| {
        let post_ids = require_str_array(args, "post_ids", cmd)?;
        Ok(("posts/get", json!({ "postIds": post_ids })))
    }
);

// #5 getPostsAfterIndex：锚 id 往后翻页。**命名陷阱**：body key `postIds` 值是**单 id string** 非数组（posts.go:318 `PostId string json:"postIds"`）。
read_cmd!(
    GetPostsAfterIndexCommand,
    GET_POSTS_AFTER_INDEX_REG,
    "im_get_posts_after_index",
    |args, cmd| {
        let post_id = require_str(args, "post_id", cmd)?;
        Ok(("posts/getPostsAfterIndex", json!({ "postIds": post_id })))
    }
);

// #6 getReplies：取某 reply 根的回复。真源 {replyId, pageNumber, pageSize, revoke?}
// （GetPostOpts 嵌 entity.PageOpts{pageNumber,pageSize}，posts.go:280）。
read_cmd!(
    GetRepliesCommand,
    GET_REPLIES_REG,
    "im_get_replies",
    |args, cmd| {
        let reply_id = require_str(args, "reply_id", cmd)?;
        Ok(("posts/getReplies", page_body(args, "replyId", reply_id)))
    }
);

// #7 getReplyBranch：取某一级回复的分支。真源 {replyFirstLevelId, pageNumber, pageSize}（posts.go:261）。
read_cmd!(
    GetReplyBranchCommand,
    GET_REPLY_BRANCH_REG,
    "im_get_reply_branch",
    |args, cmd| {
        let id = require_str(args, "reply_first_level_id", cmd)?;
        Ok((
            "posts/getReplyBranch",
            page_body(args, "replyFirstLevelId", id),
        ))
    }
);

// #8 queryTodoList：批量取待办状态。真源 {postIds:[]string} 非空（posts.go:694）。
read_cmd!(
    QueryTodoListCommand,
    QUERY_TODO_REG,
    "im_query_todo_list",
    |args, cmd| {
        let post_ids = require_str_array(args, "post_ids", cmd)?;
        Ok(("posts/queryTodoList", json!({ "postIds": post_ids })))
    }
);

// #9 getUpdatedPosts：游标拉被更新的 post。**命名陷阱**：`timeStamp` 大写 S（posts.go CursorTs tag）。
read_cmd!(
    GetUpdatedPostsCommand,
    GET_UPDATED_POSTS_REG,
    "im_get_updated_posts",
    |args, cmd| {
        let ts = args
            .get("time_stamp")
            .and_then(Value::as_i64)
            .ok_or_else(|| ImError::Parse(format!("{cmd}: 缺/坏 time_stamp（int64 毫秒）")))?;
        let limit = args.get("limit").and_then(Value::as_i64).unwrap_or(0);
        Ok((
            "posts/getUpdatedPosts",
            json!({ "timeStamp": ts, "limit": limit }),
        ))
    }
);

// #10 getLatestPost：拉某 channel 最新首屏（too_long 重拉链）。真源 {channelId, timestamp}（posts.go:201）。
read_cmd!(
    GetLatestPostCommand,
    GET_LATEST_POST_REG,
    "im_get_latest_post",
    |args, cmd| {
        let channel_id = require_str(args, "channel_id", cmd)?;
        let timestamp = args.get("timestamp").and_then(Value::as_i64).unwrap_or(0);
        Ok((
            "posts/getLatestPost",
            json!({ "channelId": channel_id, "timestamp": timestamp }),
        ))
    }
);

/// 组装分页 body：`{<id_key>: <id_val>, pageNumber, pageSize}`，可选 `revoke`（GetPostOpts.Revoke）。
/// pageNumber/pageSize 缺省 0（Go int 零值），revoke 仅在显式 bool 时下发。
fn page_body(args: &Value, id_key: &'static str, id_val: &str) -> Value {
    let page_number = args.get("page_number").and_then(Value::as_i64).unwrap_or(0);
    let page_size = args.get("page_size").and_then(Value::as_i64).unwrap_or(0);
    let mut b = json!({ id_key: id_val, "pageNumber": page_number, "pageSize": page_size });
    if let Some(revoke) = args.get("revoke").and_then(Value::as_bool) {
        b["revoke"] = json!(revoke);
    }
    b
}

// ── post/bookmark/* 读族（注意 endpoint 走 `post/`（单数）+ `/bookmark/` 子路由）────────

// #11 bookmark/create：保存书签。真源 {channelId, userId, postIds:[]string}（post_bookmark.go:54）。
read_cmd!(
    BookmarkCreateCommand,
    BOOKMARK_CREATE_REG,
    "im_bookmark_create",
    |args, cmd| {
        let channel_id = require_str(args, "channel_id", cmd)?;
        let user_id = require_str(args, "user_id", cmd)?;
        let post_ids = require_str_array(args, "post_ids", cmd)?;
        Ok((
            "post/bookmark/create",
            json!({ "channelId": channel_id, "userId": user_id, "postIds": post_ids }),
        ))
    }
);

// #12 bookmark/delete：删书签。真源 {userId, postId}（post_bookmark.go:36）。
read_cmd!(
    BookmarkDeleteCommand,
    BOOKMARK_DELETE_REG,
    "im_bookmark_delete",
    |args, cmd| {
        let user_id = require_str(args, "user_id", cmd)?;
        let post_id = require_str(args, "post_id", cmd)?;
        Ok((
            "post/bookmark/delete",
            json!({ "userId": user_id, "postId": post_id }),
        ))
    }
);

// #13 bookmark/load：拉书签列表。真源 {channelId, userId}（post_bookmark.go:17）。
read_cmd!(
    BookmarkLoadCommand,
    BOOKMARK_LOAD_REG,
    "im_bookmark_load",
    |args, cmd| {
        let channel_id = require_str(args, "channel_id", cmd)?;
        let user_id = require_str(args, "user_id", cmd)?;
        Ok((
            "post/bookmark/load",
            json!({ "channelId": channel_id, "userId": user_id }),
        ))
    }
);

// ── post/announcement/* 读族（走 `post/`（单数）子路由）────────────────────────────

// #14 announcement/acceptList：公告接收列表。真源 {postId}（posts.go AnnouncementAcceptList）。
read_cmd!(
    AnnounceAcceptListCommand,
    ANNOUNCE_ACCEPT_LIST_REG,
    "im_announcement_accept_list",
    |args, cmd| {
        let post_id = require_str(args, "post_id", cmd)?;
        Ok(("post/announcement/acceptList", json!({ "postId": post_id })))
    }
);

// #15 announcement/list：公告列表。真源 {channelId,postId?}（AnnouncementList：channelId 空回退 postId，前端实测 postId 实传 channelId 见 公告功能-前端联调.md:214）。
read_cmd!(
    AnnounceListCommand,
    ANNOUNCE_LIST_REG,
    "im_announcement_list",
    |args, cmd| {
        let channel_id = require_str(args, "channel_id", cmd)?;
        let mut b = json!({ "channelId": channel_id });
        if let Some(post_id) = args.get("post_id").and_then(Value::as_str) {
            b["postId"] = json!(post_id);
        }
        Ok(("post/announcement/list", b))
    }
);

// #16 announcement/detail：公告详情（仅公告表）。真源 {postIds:[]string}（posts.go AnnouncementDetail）。
read_cmd!(
    AnnounceDetailCommand,
    ANNOUNCE_DETAIL_REG,
    "im_announcement_detail",
    |args, cmd| {
        let post_ids = require_str_array(args, "post_ids", cmd)?;
        Ok(("post/announcement/detail", json!({ "postIds": post_ids })))
    }
);

// ── post/read/list 已读位历史读（B1：非 bitmap，走 post/ 单数）───────────────────────

// #17 post/read/list：批量取 post 已读位历史。真源 entity.PostReadListParam{postIds:[]string}
// （post.go:554，postReadList → post/read/list 路由 posts.go:31）。
read_cmd!(
    PostReadListCommand,
    POST_READ_LIST_REG,
    "im_post_read_list",
    |args, cmd| {
        let post_ids = require_str_array(args, "post_ids", cmd)?;
        Ok(("post/read/list", json!({ "postIds": post_ids })))
    }
);
