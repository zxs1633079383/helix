//! posts 读族 outbound 扩展（bookmark / announcement / read/list）——从 posts_read.rs 外提 sibling。
//!
//! 与 `posts_read.rs` 分文件（§1 单文件 ≤300 行硬顶；posts_read.rs 已满）。
//! 注册方式与 posts_read.rs 完全相同（`inventory` + 本地 `read_cmd!` 宏）。
//! endpoint / body 真源同 posts_read.rs 头注释（mattermost csesapi 逐字，不自造 fixture）。

use serde_json::{json, Value};

use crate::error::ImError;
use crate::outbound::registry::{require_str, OutboundCommand, OutboundRegistration};
// helpers 经 pub(super) 从 posts_read 引入
use super::read::require_str_array;

/// 校验批量回执查询的公开边界，只允许 `post_ids` 与宿主追踪键 `req_id`。
fn require_post_read_list_args(args: &Value, cmd: &str) -> Result<Value, ImError> {
    let object = args
        .as_object()
        .ok_or_else(|| ImError::Parse(format!("{cmd}: payload 必须是 JSON 对象")))?;
    for key in object.keys() {
        if !matches!(key.as_str(), "post_ids" | "req_id") {
            return Err(ImError::Parse(format!("{cmd}: 未知或内部字段 '{key}'")));
        }
    }
    if let Some(req_id) = object.get("req_id") {
        if !req_id.is_string() {
            return Err(ImError::Parse(format!("{cmd}: req_id 必须是字符串")));
        }
    }
    require_str_array(args, "post_ids", cmd)
}

/// 拒绝书签命令的 user/offset 等内部字段，冻结 Helix 到最小公开边界。
fn require_bookmark_keys(args: &Value, allowed: &[&str], cmd: &str) -> Result<(), ImError> {
    let object = args
        .as_object()
        .ok_or_else(|| ImError::Parse(format!("{cmd}: payload 必须是 JSON 对象")))?;
    if let Some(unknown) = object.keys().find(|key| !allowed.contains(&key.as_str())) {
        return Err(ImError::Parse(format!("{cmd}: 未知或内部字段 '{unknown}'")));
    }
    if let Some(req_id) = object.get("req_id") {
        if req_id.as_str().is_none_or(str::is_empty) {
            return Err(ImError::Parse(format!("{cmd}: req_id 必须是非空字符串")));
        }
    }
    Ok(())
}

/// 读取 1-based bookmark 页参数；offset 不属于公开命令，因此不在此处计算或接收。
fn require_positive_page(args: &Value, key: &str, cmd: &str) -> Result<i64, ImError> {
    args.get(key)
        .and_then(Value::as_i64)
        .filter(|value| *value > 0)
        .ok_or_else(|| ImError::Parse(format!("{cmd}: {key} 必须是正整数")))
}

/// 公告列表只允许 channel、可选 post 兼容键和请求关联键。
fn require_announcement_list_keys(args: &Value, cmd: &str) -> Result<(), ImError> {
    let object = args
        .as_object()
        .ok_or_else(|| ImError::Parse(format!("{cmd}: payload 必须是 JSON 对象")))?;
    if let Some(unknown) = object
        .keys()
        .find(|key| !matches!(key.as_str(), "channel_id" | "post_id" | "req_id"))
    {
        return Err(ImError::Parse(format!("{cmd}: 未知或内部字段 '{unknown}'")));
    }
    if let Some(req_id) = object.get("req_id") {
        if req_id.as_str().is_none_or(str::is_empty) {
            return Err(ImError::Parse(format!("{cmd}: req_id 必须是非空字符串")));
        }
    }
    Ok(())
}

/// 把 Go 公告列表回包归一为带版本的绝对集合；Go 初始频道允许权威 version=0。
pub fn announcement_list_projection(channel_id: &str, body: &Value) -> Option<Value> {
    let payload = body
        .get("data")
        .filter(|value| value.is_object())
        .unwrap_or(body);
    let version = payload.get("version").and_then(Value::as_u64)?;
    let source_channel_id = payload
        .get("channelId")
        .or_else(|| payload.get("channel_id"))
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .unwrap_or(channel_id);
    if source_channel_id != channel_id {
        return None;
    }
    let items = payload.get("announcements").and_then(Value::as_array)?;
    let announcements = items
        .iter()
        .map(announcement_projection)
        .collect::<Option<Vec<_>>>()?;
    Some(json!({
        "channelId": channel_id,
        "version": version,
        "announcements": announcements,
    }))
}

/// 归一公告删除的结构化回执；version=0 或显式 noOp 是合法幂等未命中。
pub fn announcement_delete_projection(channel_id: &str, body: &Value) -> Option<Value> {
    let payload = body
        .get("data")
        .filter(|value| value.is_object())
        .unwrap_or(body);
    let result_channel_id = payload
        .get("channelId")
        .or_else(|| payload.get("channel_id"))
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())?;
    if result_channel_id != channel_id {
        return None;
    }
    let deleted_id = payload
        .get("deletedAnnouncementId")
        .or_else(|| payload.get("deleted_announcement_id"))
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())?;
    let version = payload.get("version").and_then(Value::as_u64)?;
    let explicit_no_op = payload
        .get("noOp")
        .or_else(|| payload.get("no_op"))
        .or_else(|| payload.get("noop"))
        .map(Value::as_bool);
    if explicit_no_op.is_some_and(|value| value.is_none()) {
        return None;
    }
    let no_op = version == 0 || explicit_no_op.flatten().unwrap_or(false);
    let mut result = json!({
        "channelId": channel_id,
        "deletedAnnouncementId": deleted_id,
        "version": version,
    });
    if no_op {
        result["noOp"] = Value::Bool(true);
    }
    Some(result)
}

/// 将一条 Go 公告记录压缩为 Angular 所需的绝对字段；缺主键时拒绝整份快照。
fn announcement_projection(item: &Value) -> Option<Value> {
    let announcement_id = item
        .get("announcementId")
        .or_else(|| item.get("announcement_id"))
        .or_else(|| item.get("id"))
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())?;
    let post_id = item
        .get("postId")
        .or_else(|| item.get("post_id"))
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())?;
    let channel_id = item
        .get("channelId")
        .or_else(|| item.get("channel_id"))
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())?;
    let content = item
        .get("content")
        .or_else(|| item.get("message"))
        .or_else(|| item.get("simpleMessage"))
        .and_then(Value::as_str)?;
    let create_at = item
        .get("createAt")
        .or_else(|| item.get("create_at"))
        .and_then(Value::as_i64)
        .or_else(|| {
            item.get("createAt")
                .or_else(|| item.get("create_at"))
                .and_then(Value::as_u64)
                .and_then(|value| i64::try_from(value).ok())
        })?;
    let create_by = item
        .get("createBy")
        .or_else(|| item.get("create_by"))
        .or_else(|| item.get("userId"))
        .and_then(Value::as_str)?;
    Some(json!({
        "announcementId": announcement_id,
        "postId": post_id,
        "channelId": channel_id,
        "content": content,
        "createAt": create_at,
        "createBy": create_by,
    }))
}

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
        inventory::submit! {
            OutboundRegistration {
                name: $name,
                command: &$cmd_struct,
            }
        }
    };
}

// ── post/bookmark/* 读族（注意 endpoint 走 `post/`（单数）+ `/bookmark/` 子路由）────────

// #11 bookmark/create：保存书签。actor 由 Go session.currentUserId 派生，wire 只含 channel/post。
read_cmd!(
    BookmarkCreateCommand,
    BOOKMARK_CREATE_REG,
    "im_bookmark_create",
    |args, cmd| {
        require_bookmark_keys(args, &["channel_id", "post_ids", "req_id"], cmd)?;
        let channel_id = require_str(args, "channel_id", cmd)?;
        let post_ids = require_str_array(args, "post_ids", cmd)?;
        Ok((
            "post/bookmark/create",
            json!({ "channelId": channel_id, "postIds": post_ids }),
        ))
    }
);

#[cfg(test)]
mod tests {
    use super::*;

    /// 公告 list 回包必须带非负版本，并归一为固定的频道绝对集合。
    #[test]
    fn announcement_list_projection_requires_version_and_canonical_fields() {
        let body = json!({
            "status": "SUCCESS",
            "data": {
                "channelId": "c1",
                "version": 3,
                "announcements": [{
                    "announcementId": "a1",
                    "postId": "p1",
                    "channelId": "c1",
                    "content": "公告",
                    "createAt": 42,
                    "createBy": "u1",
                    "extra": "discarded"
                }]
            }
        });
        assert_eq!(
            announcement_list_projection("c1", &body),
            Some(json!({
                "channelId": "c1",
                "version": 3,
                "announcements": [{
                    "announcementId": "a1",
                    "postId": "p1",
                    "channelId": "c1",
                    "content": "公告",
                    "createAt": 42,
                    "createBy": "u1"
                }]
            }))
        );
        assert!(announcement_list_projection(
            "c1",
            &json!({
                "data": { "channelId": "c1", "announcements": [] }
            })
        )
        .is_none());
        assert!(announcement_list_projection(
            "c1",
            &json!({
                "data": {
                    "channelId": "c1",
                    "version": 3,
                    "announcements": [{ "postId": "p1" }]
                }
            })
        )
        .is_none());
        assert_eq!(
            announcement_list_projection(
                "c1",
                &json!({
                    "data": { "channelId": "c1", "version": 0, "announcements": [] }
                })
            ),
            Some(json!({ "channelId": "c1", "version": 0, "announcements": [] }))
        );
    }

    /// 公告 delete 的 Go 初始版本零与显式 noOp 都归一为成功幂等结果。
    #[test]
    fn announcement_delete_projection_accepts_version_zero_and_explicit_noop() {
        assert_eq!(
            announcement_delete_projection(
                "c1",
                &json!({
                    "data": {
                        "channelId": "c1",
                        "deletedAnnouncementId": "missing",
                        "version": 0
                    }
                })
            ),
            Some(json!({
                "channelId": "c1",
                "deletedAnnouncementId": "missing",
                "version": 0,
                "noOp": true
            }))
        );
        assert_eq!(
            announcement_delete_projection(
                "c1",
                &json!({
                    "channelId": "c1",
                    "deletedAnnouncementId": "already-gone",
                    "version": 4,
                    "no_op": true
                })
            )
            .and_then(|result| result.get("noOp").cloned()),
            Some(Value::Bool(true))
        );
        assert_eq!(
            announcement_delete_projection(
                "c1",
                &json!({
                    "channelId": "c1",
                    "deletedAnnouncementId": "go-noop",
                    "version": 0,
                    "noop": true
                })
            )
            .and_then(|result| result.get("noOp").cloned()),
            Some(Value::Bool(true))
        );
        assert_eq!(
            announcement_delete_projection(
                "c1",
                &json!({
                    "channelId": "c1",
                    "deletedAnnouncementId": "version-zero",
                    "version": 0,
                    "noOp": false
                })
            )
            .and_then(|result| result.get("noOp").cloned()),
            Some(Value::Bool(true))
        );
        assert!(announcement_delete_projection(
            "c1",
            &json!({
                "channelId": "c1",
                "deletedAnnouncementId": "bad-marker",
                "version": 0,
                "noOp": "true"
            })
        )
        .is_none());
    }

    /// 公告 delete 回包禁止跨频道结果伪装成本频道成功。
    #[test]
    fn announcement_delete_projection_rejects_wrong_channel() {
        let body = json!({
            "data": {
                "channelId": "c2",
                "deletedAnnouncementId": "a1",
                "version": 4
            }
        });
        assert!(announcement_delete_projection("c1", &body).is_none());
    }
}

// #12 bookmark/delete：删书签。channelId 与 postId 同时进入 wire，避免跨频道误删。
read_cmd!(
    BookmarkDeleteCommand,
    BOOKMARK_DELETE_REG,
    "im_bookmark_delete",
    |args, cmd| {
        require_bookmark_keys(args, &["channel_id", "post_id", "req_id"], cmd)?;
        let channel_id = require_str(args, "channel_id", cmd)?;
        let post_id = require_str(args, "post_id", cmd)?;
        Ok((
            "post/bookmark/delete",
            json!({ "channelId": channel_id, "postId": post_id }),
        ))
    }
);

// #13 bookmark/load：拉书签列表；Go 从 1-based pageNumber/pageSize 计算内部 offset。
read_cmd!(
    BookmarkLoadCommand,
    BOOKMARK_LOAD_REG,
    "im_bookmark_load",
    |args, cmd| {
        require_bookmark_keys(
            args,
            &["channel_id", "page_number", "page_size", "req_id"],
            cmd,
        )?;
        let channel_id = require_str(args, "channel_id", cmd)?;
        let page_number = require_positive_page(args, "page_number", cmd)?;
        let page_size = require_positive_page(args, "page_size", cmd)?;
        Ok((
            "post/bookmark/load",
            json!({
                "channelId": channel_id,
                "pageNumber": page_number,
                "pageSize": page_size,
            }),
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

// #15 announcement/list：公告列表。真源 {channelId,postId?}（前端实测 postId 实传 channelId）。
read_cmd!(
    AnnounceListCommand,
    ANNOUNCE_LIST_REG,
    "im_announcement_list",
    |args, cmd| {
        require_announcement_list_keys(args, cmd)?;
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

// ── post/read/list receipt snapshot 读（走 post/ 单数）──────────────────────────────

// #17 post/read/list：批量取 post receipt snapshot。真源 entity.PostReadListParam{postIds:[]string}
// （post.go:554，postReadList → post/read/list 路由 posts.go:31）；Helix 后续只投影有序 ID、
// 绝对计数和 snapshotId，Go 内部 bitmap 不进入公开事件。
read_cmd!(
    PostReadListCommand,
    POST_READ_LIST_REG,
    "im_post_read_list",
    |args, cmd| {
        let post_ids = require_post_read_list_args(args, cmd)?;
        Ok(("post/read/list", json!({ "postIds": post_ids })))
    }
);

// 全局书签单独查询；认证 actor/company 由会话注入，不能由 renderer 覆盖。
read_cmd!(
    BookmarkLoadGlobalCommand,
    BOOKMARK_LOAD_GLOBAL_REG,
    "im_bookmark_load_global",
    |args, cmd| {
        require_bookmark_keys(args, &["page_number", "page_size", "req_id"], cmd)?;
        let page_number = require_positive_page(args, "page_number", cmd)?;
        let page_size = require_positive_page(args, "page_size", cmd)?;
        if page_size > 100 || page_number > 1_000_000 {
            return Err(ImError::Parse(format!("{cmd}: pagination out of range")));
        }
        Ok((
            "post/bookmark/global",
            json!({"pageNumber":page_number,"pageSize":page_size}),
        ))
    }
);
