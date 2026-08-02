//! posts 读族 outbound 命令（P3a，parallelGroup=G1）。
//!
//! 纯**读** request/response：helix-im 只 build 正确 wire body。读族无 WS 回声、HTTP 响应体即数据
//! → `is_read=true`（spec06 缺陷A），dispatch 注册 `OutboundReadReply` 把响应透传回灌前端（read_relay）。
//! 与 P3b 真零交集：独立文件 + inventory 注册，不碰 channel*/registry.rs（仅 mod.rs append）。
//! ## endpoint / body 真源（mattermost csesapi 逐字，不自造 fixture，C5）
//! - posts/* 子路由 = `api/cses/posts`（api.go:169）；post/* = `api/cses/post`（api.go:171）；
//!   post/bookmark/* = `api/cses/post/bookmark`（api.go:177）。
//! - 命名陷阱（decode 静默失败防线）：top20 → **snake_case** `channel_id`（posts.go:550）；
//!   getPostsAfterIndex → `postIds` **单数 string**（posts.go:318，非数组）；
//!   getUpdatedPosts → `timeStamp` **大写 S**（posts.go:体 CursorTs tag）；时间 `createAt`。

use serde_json::{json, Value};
use std::collections::HashSet;

use crate::error::ImError;

use crate::outbound::registry::{require_str, OutboundCommand, OutboundRegistration};

// ── 本文件局部 helper（边界零信任：缺/类型错 → Err，不 panic）──────────────────────

/// 取必填字符串数组（非空、元素全 string）。供 `{postIds:[]}` 类命令复用（含 posts_read_ext）。
pub(super) fn require_str_array(args: &Value, key: &str, cmd: &str) -> Result<Value, ImError> {
    args.get(key)
        .and_then(Value::as_array)
        .filter(|a| !a.is_empty() && a.iter().all(Value::is_string))
        .cloned()
        .map(Value::Array)
        .ok_or_else(|| ImError::Parse(format!("{cmd}: 缺/空 {key}（非空字符串数组）")))
}

/// 解析 G11g exact-by-id 请求；空数组合法，并按首次出现顺序去重保证幂等。
pub(crate) fn exact_post_ids(args: &Value, cmd: &str) -> Result<Vec<String>, ImError> {
    let values = args
        .get("post_ids")
        .and_then(Value::as_array)
        .ok_or_else(|| ImError::Parse(format!("{cmd}: 缺 post_ids（字符串数组）")))?;
    let mut seen = HashSet::new();
    let mut ids = Vec::with_capacity(values.len());
    for value in values {
        let id = value
            .as_str()
            .filter(|id| !id.is_empty())
            .ok_or_else(|| ImError::Parse(format!("{cmd}: post_ids 必须为非空字符串数组")))?;
        if seen.insert(id.to_string()) {
            ids.push(id.to_string());
        }
    }
    Ok(ids)
}

/// 把入参 `in_key`（snake_case）原样透传成 wire 字段 `wire_key`（camelCase），仅当存在且非 null。
/// 不强转类型——边界零信任只校验「存在」，数值/字符串原值由前端 bridge 保证（对齐 vote_score::carry_optional）。
pub(super) fn carry_into(
    body: &mut serde_json::Map<String, Value>,
    args: &Value,
    in_key: &str,
    wire_key: &str,
) {
    if let Some(v) = args.get(in_key) {
        if !v.is_null() {
            body.insert(wire_key.to_string(), v.clone());
        }
    }
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
        inventory::submit! {
            OutboundRegistration {
                name: $name,
                command: &$cmd_struct,
            }
        }
    };
}

// ── posts/* 读族 ────────────────────────────────────────────────────────────────

// #1 getSchedule：查询当前 session owner 的定时消息。Go 从 session 派生 owner，客户端只发 channelId；
// 旧 user_id 可留在输入对象中兼容历史调用，但不得进入 wire。
read_cmd!(
    GetScheduleCommand,
    GET_SCHEDULE_REG,
    "im_get_schedule",
    |args, cmd| {
        let channel_id = require_str(args, "channel_id", cmd)?;
        Ok(("posts/getSchedule", json!({ "channelId": channel_id })))
    }
);

/// MV3-G04d：把 `posts/getSchedule` 的服务端响应体归一为**稳定排序的绝对 `ScheduleProjection[]`**。
///
/// 权威参考包 `gates/schedule/mv3-g04d/outbound-sample.json` 的 `command_results[0].rust`：
/// `{"ok": true, "schedules": [{id, channelId, executeAt, status, post:{type, message, viewers}}]}`。
///
/// ## 排序（合同 `outbound-contract.json`：「查询结果按 executeAt 稳定排序」）
///
/// 主键 `executeAt` 升序；`executeAt` 相同再按 schedule id 升序 —— 服务端返回顺序不稳定时
/// 也保证同一份数据永远产出同一个数组（重复到达 → Angular 内存与 DOM 不变，合同 §5 幂等）。
///
/// ## 公开结果
///
/// Phase 1 合同决议固定 Angular `ScheduleProjection` 的唯一键集：`scheduleId`、
/// `post.text`、`post.channelId`。旧 `id` / `post.message` 不再双写，避免 WDIO 和业务调用方
/// 在两套等价值之间产生不稳定 fixture。
///
/// ## 边界零信任
///
/// 无待发对象（`null` / `[]` / 缺字段）→ 空数组，**绝不抛错**（合同：读取失败不得拖垮切会话）。
pub fn schedule_projections(channel_id: &str, body: &Value) -> Value {
    let mut items: Vec<(i64, String, Value)> = schedule_items(body)
        .iter()
        .filter_map(|item| schedule_projection(channel_id, item))
        .collect();
    // Rust 的 sort_by 是稳定排序：同 (executeAt, id) 的元素保持服务端原序。
    items.sort_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.cmp(&right.1)));
    Value::Array(items.into_iter().map(|(_, _, item)| item).collect())
}

/// 从任意合法响应外壳里取出待发对象列表（`{data:[]}` / `{data:{}}` / `[]` / `{...}`
/// / `{data:{schedules:[]}}`）。
///
/// 最后一种是 Go 常见的双层信封（业务 `{status,data}` 外壳里再包一层带列表键的 payload）。
/// 不识别它的话，`{data:{schedules:[…]}}` 会被当成**一条**缺 `executeAt` 的脏对象整条丢掉 ——
/// 表现为「定时设上了但切回会话提示条永远不出现」，用户以为没设上而重复设定，到点多发
/// （contract.json `openDecisions.impact` 描述的正是这个故障）。
fn schedule_items(body: &Value) -> Vec<Value> {
    let payload = body
        .get("data")
        .or_else(|| body.get("schedules"))
        .unwrap_or(body);
    match payload {
        Value::Array(items) => items.clone(),
        Value::Object(item) => match nested_list(item) {
            // 内层列表键命中 → 用内层列表；否则整个对象就是单条待发对象。
            Some(items) => items.clone(),
            None if !item.is_empty() => vec![Value::Object(item.clone())],
            None => Vec::new(),
        },
        _ => Vec::new(),
    }
}

/// 对象内层的列表键（只认数组，避免把业务对象误当列表）。
fn nested_list(item: &serde_json::Map<String, Value>) -> Option<&Vec<Value>> {
    ["schedules", "list", "items", "records"]
        .iter()
        .find_map(|key| item.get(*key).and_then(Value::as_array))
}

/// 单条待发对象 → 绝对投影；缺 `executeAt` 锚点的脏数据直接丢弃（不进不稳定排序）。
fn schedule_projection(channel_id: &str, item: &Value) -> Option<(i64, String, Value)> {
    let execute_at = first_i64(
        item,
        &[
            "executeAt",
            "execute_at",
            "schedulePostAt",
            "schedule_post_at",
            "sendAt",
            "send_at",
        ],
    )?;
    let schedule_id = first_str(item, &["scheduleId", "schedule_id", "id"])
        .unwrap_or_default()
        .to_string();
    let channel = first_str(item, &["channelId", "channel_id"])
        .unwrap_or(channel_id)
        .to_string();
    let status = first_str(item, &["status"])
        .filter(|status| !status.is_empty())
        .unwrap_or("scheduled")
        .to_string();
    let message = first_str(item, &["message", "text"]).unwrap_or_default();
    let post = json!({
        "channelId": channel,
        "type": first_str(item, &["type", "msgType", "msg_type"]).unwrap_or("TEXT"),
        "text": message,
        "viewers": item
            .get("viewers")
            .filter(|viewers| viewers.is_array())
            .cloned()
            .unwrap_or_else(|| json!(["all"])),
        "props": item
            .get("props")
            .filter(|props| props.is_object())
            .cloned()
            .unwrap_or_else(|| json!({})),
    });
    let projection = json!({
        "scheduleId": schedule_id,
        "channelId": channel,
        "executeAt": execute_at,
        "status": status,
        "post": post,
    });
    Some((execute_at, schedule_id, projection))
}

/// 按候选键序取第一个字符串值。
fn first_str<'a>(item: &'a Value, keys: &[&str]) -> Option<&'a str> {
    keys.iter()
        .find_map(|key| item.get(*key).and_then(Value::as_str))
}

/// 按候选键序取第一个整数值（毫秒时间戳）。
fn first_i64(item: &Value, keys: &[&str]) -> Option<i64> {
    keys.iter()
        .find_map(|key| item.get(*key).and_then(Value::as_i64))
}

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
        let post_ids = exact_post_ids(args, cmd)?;
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
        let mut body = json!({ "channelId": channel_id, "timestamp": timestamp });
        if let Some(cursor_version) = args.get("cursor_version").and_then(Value::as_u64) {
            body["cursorVersion"] = Value::from(cursor_version);
        }
        if let Some(page_size) = args.get("page_size").and_then(Value::as_u64) {
            body["pageSize"] = Value::from(page_size);
        }
        Ok(("posts/getLatestPost", body))
    }
);

/// 组装分页 body：`{<id_key>: <id_val>, pageNumber, pageSize}`，可选 `revoke`（GetPostOpts.Revoke）。
/// pageNumber/pageSize 缺省 1/20（与 Go PageOpts.Init 一致）。Go 的 reply branch handler 不主动
/// Init；若传 0，会在“多取一条判 hasMore”后截断为 0 条，因此默认值必须由 Helix 业务层补齐。
fn page_body(args: &Value, id_key: &'static str, id_val: &str) -> Value {
    let page_number = args
        .get("page_number")
        .and_then(Value::as_i64)
        .filter(|value| *value > 0)
        .unwrap_or(1);
    let page_size = args
        .get("page_size")
        .and_then(Value::as_i64)
        .filter(|value| *value > 0)
        .unwrap_or(20);
    let mut b = json!({ id_key: id_val, "pageNumber": page_number, "pageSize": page_size });
    if let Some(revoke) = args.get("revoke").and_then(Value::as_bool) {
        b["revoke"] = json!(revoke);
    }
    b
}

#[cfg(test)]
mod tests {
    use super::{exact_post_ids, page_body};
    use serde_json::json;

    #[test]
    fn reply_page_defaults_match_go_page_opts() {
        let body = page_body(&json!({}), "replyFirstLevelId", "first-1");
        assert_eq!(body["replyFirstLevelId"], "first-1");
        assert_eq!(body["pageNumber"], 1);
        assert_eq!(body["pageSize"], 20);
    }

    #[test]
    fn reply_page_rejects_non_positive_values_and_keeps_positive_values() {
        let defaults = page_body(
            &json!({"page_number": 0, "page_size": -1}),
            "replyId",
            "root-1",
        );
        assert_eq!(defaults["pageNumber"], 1);
        assert_eq!(defaults["pageSize"], 20);

        let explicit = page_body(
            &json!({"page_number": 2, "page_size": 50}),
            "replyId",
            "root-1",
        );
        assert_eq!(explicit["pageNumber"], 2);
        assert_eq!(explicit["pageSize"], 50);
    }

    /// G11g 空请求是合法零结果；重复 id 只保留首次出现以保证重放幂等。
    #[test]
    fn exact_post_ids_accepts_empty_and_deduplicates() {
        assert_eq!(
            exact_post_ids(&json!({"post_ids": []}), "im_get_posts").unwrap(),
            Vec::<String>::new()
        );
        assert_eq!(
            exact_post_ids(&json!({"post_ids": ["p1", "p1", "p2"]}), "im_get_posts").unwrap(),
            vec!["p1", "p2"]
        );
    }
}

// ── post/bookmark/* / announcement/* / post/read/list 读族 → posts_read_ext.rs ──────
