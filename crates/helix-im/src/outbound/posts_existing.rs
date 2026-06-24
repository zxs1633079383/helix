//! 既有 posts 域 outbound 命令（P2 registry 化迁移：read / revoke / schedule / cancel）。
//!
//! 行为/endpoint/body 真源逐条标注（`full-map/partials/1-*.md` + `真机curl真源.md`）。
//! 从旧 `commands.rs::handle_outbound` 单 match 抽出，逐命令 distributed_slice 注册。

use linkme::distributed_slice;
use serde_json::{json, Value};

use crate::error::ImError;

use super::registry::{require_str, OutboundCommand, OutboundRegistration, OUTBOUND_COMMANDS};

/// #23 POST /api/cses/post/read — 标记已读 → update_channel + post_read。
/// 真源 `entity.PostRead{ChannelID, EndTime, Posts[], StartTime, UserId}`（post.go:527-536）。
/// 两种已读模式：① 按 `posts` 列表标记单条（UC-3.2）② 按 `channelId + 区间(start/end)`。
/// `UserId` nil → Go 用 session 兜底。
///
/// ⚠️ UC-3.2 偏差修复：前端 `onPostRead` 发 `{channel_id, posts:[postId]}`（message.service.ts:247）
/// 走 posts 列表模式标单条已读；旧 builder 从不构建 `posts` → Go ReadPost 收无 posts 无区间 →
/// 标记范围错/不标记。`posts` 为 wire key（entity.PostRead.Posts `json:"posts,omitempty"`）。
struct PostReadCommand;
impl OutboundCommand for PostReadCommand {
    fn name(&self) -> &'static str {
        "im_post_read"
    }
    fn build(&self, args: &Value) -> Result<(&'static str, Value), ImError> {
        let channel_id = require_str(args, "channel_id", self.name())?;
        let mut b = json!({ "channelId": channel_id });
        if let Some(end_time) = args.get("end_time").and_then(Value::as_i64) {
            b["endTime"] = json!(end_time);
        }
        if let Some(start_time) = args.get("start_time").and_then(Value::as_i64) {
            b["startTime"] = json!(start_time);
        }
        // posts 列表模式：仅在非空字符串数组时注入（空/非数组 → 走区间模式，不发坏 wire）。
        if let Some(arr) = args.get("posts").and_then(Value::as_array) {
            let posts: Vec<&str> = arr.iter().filter_map(Value::as_str).collect();
            if !posts.is_empty() && posts.len() == arr.len() {
                b["posts"] = json!(posts);
            }
        }
        Ok(("post/read", b))
    }
}
static POST_READ: PostReadCommand = PostReadCommand;
#[distributed_slice(OUTBOUND_COMMANDS)]
static POST_READ_REG: OutboundRegistration = OutboundRegistration {
    name: "im_post_read",
    command: &POST_READ,
};

/// #14 POST /api/cses/posts/revoke — 撤回 → post_update。真源 {PostId}（真机curl真源 §3）。
struct RevokeCommand;
impl OutboundCommand for RevokeCommand {
    fn name(&self) -> &'static str {
        "im_revoke"
    }
    fn build(&self, args: &Value) -> Result<(&'static str, Value), ImError> {
        let post_id = require_str(args, "post_id", self.name())?;
        Ok(("posts/revoke", json!({ "postId": post_id })))
    }
}
static REVOKE: RevokeCommand = RevokeCommand;
#[distributed_slice(OUTBOUND_COMMANDS)]
static REVOKE_REG: OutboundRegistration = OutboundRegistration {
    name: "im_revoke",
    command: &REVOKE,
};

/// #3 POST /api/cses/posts/createSchedule — 定时消息 → post_schedule_created。
/// 真源 CreateSchedulePostReq{Post *Post, ScheduledTime int64 `json:"schedulePostAt"`}。
struct CreateScheduleCommand;
impl OutboundCommand for CreateScheduleCommand {
    fn name(&self) -> &'static str {
        "im_create_schedule"
    }
    fn build(&self, args: &Value) -> Result<(&'static str, Value), ImError> {
        let channel_id = require_str(args, "channel_id", self.name())?;
        let message = require_str(args, "message", self.name())?;
        let schedule_post_at = args
            .get("schedule_post_at")
            .and_then(Value::as_i64)
            .ok_or_else(|| {
                ImError::Parse(format!(
                    "{}: 缺/坏 schedule_post_at（int64 毫秒）",
                    self.name()
                ))
            })?;
        let mut post = json!({ "channelId": channel_id, "message": message });
        if let Some(tmp) = args.get("temporary_id").and_then(Value::as_str) {
            post["temporaryId"] = json!(tmp);
        }
        Ok((
            "posts/createSchedule",
            json!({ "post": post, "schedulePostAt": schedule_post_at }),
        ))
    }
}
static CREATE_SCHEDULE: CreateScheduleCommand = CreateScheduleCommand;
#[distributed_slice(OUTBOUND_COMMANDS)]
static CREATE_SCHEDULE_REG: OutboundRegistration = OutboundRegistration {
    name: "im_create_schedule",
    command: &CREATE_SCHEDULE,
};

/// #4 POST /api/cses/posts/cancelSchedule — 取消定时 → post_schedule_canceled。
/// 真源 CancelSchedulePostReq{ChannelId}；userId 取 session。
struct CancelScheduleCommand;
impl OutboundCommand for CancelScheduleCommand {
    fn name(&self) -> &'static str {
        "im_cancel_schedule"
    }
    fn build(&self, args: &Value) -> Result<(&'static str, Value), ImError> {
        let channel_id = require_str(args, "channel_id", self.name())?;
        Ok(("posts/cancelSchedule", json!({ "channelId": channel_id })))
    }
}
static CANCEL_SCHEDULE: CancelScheduleCommand = CancelScheduleCommand;
#[distributed_slice(OUTBOUND_COMMANDS)]
static CANCEL_SCHEDULE_REG: OutboundRegistration = OutboundRegistration {
    name: "im_cancel_schedule",
    command: &CANCEL_SCHEDULE,
};
