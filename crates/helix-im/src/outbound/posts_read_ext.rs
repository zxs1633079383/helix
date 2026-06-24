//! posts 读族 outbound 扩展（bookmark / announcement / read/list）——从 posts_read.rs 外提 sibling。
//!
//! 与 `posts_read.rs` 分文件（§1 单文件 ≤300 行硬顶；posts_read.rs 已满）。
//! 注册方式与 posts_read.rs 完全相同（`distributed_slice` + 本地 `read_cmd!` 宏）。
//! endpoint / body 真源同 posts_read.rs 头注释（mattermost csesapi 逐字，不自造 fixture）。

use linkme::distributed_slice;
use serde_json::{json, Value};

use super::registry::{require_str, OutboundCommand, OutboundRegistration, OUTBOUND_COMMANDS};
use crate::error::ImError;
// helpers 经 pub(super) 从 posts_read 引入
use super::posts_read::{carry_into, require_str_array};

/// 注册读命令样板（与 posts_read.rs 的 read_cmd! 同构，is_read=true）。
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
            fn is_read(&self) -> bool {
                true
            }
        }
        #[distributed_slice(OUTBOUND_COMMANDS)]
        static $reg: OutboundRegistration = OutboundRegistration {
            name: $name,
            command: &$cmd_struct,
        };
    };
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

// #13 bookmark/load：拉书签列表。真源 {channelId, userId} + 内嵌 entity.PageOpts（扁平序列化）。
// pageSize 缺失 → SQL LIMIT≈0/1 → 书签面板最多回 1 条 + hasMore 失真 + 无限滚动断。
read_cmd!(
    BookmarkLoadCommand,
    BOOKMARK_LOAD_REG,
    "im_bookmark_load",
    |args, cmd| {
        let channel_id = require_str(args, "channel_id", cmd)?;
        let user_id = require_str(args, "user_id", cmd)?;
        let mut body = serde_json::Map::new();
        body.insert("channelId".to_string(), json!(channel_id));
        body.insert("userId".to_string(), json!(user_id));
        carry_into(&mut body, args, "page_size", "pageSize");
        carry_into(&mut body, args, "page_number", "pageNumber");
        carry_into(&mut body, args, "offset", "offset");
        Ok(("post/bookmark/load", Value::Object(body)))
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

// #15 announcement/list：公告列表。真源 {channelId,postId?}（前端实测 postId 实传 channelId）。
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

// #16 announcement/detail：公告详情。真源 {postIds:[]string}（posts.go AnnouncementDetail）。
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
