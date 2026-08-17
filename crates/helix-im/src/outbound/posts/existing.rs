//! 既有 posts 域 outbound 命令（P2 registry 化迁移：read / revoke / schedule / cancel）。
//!
//! 行为/endpoint/body 真源逐条标注（`full-map/partials/1-*.md` + `真机curl真源.md`）。
//! 从旧 `commands.rs::handle_outbound` 单 match 抽出，逐命令 inventory 注册。

use serde_json::{json, Value};

use crate::error::ImError;

use crate::outbound::registry::{require_str, OutboundCommand, OutboundRegistration};

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
    /// 将已归一化的 canonical 已读意图冻结为唯一 Go `/post/read` body。
    fn build(&self, args: &Value) -> Result<(&'static str, Value), ImError> {
        require_exact_keys(args, &["channel_id", "posts"], self.name())?;
        let channel_id = require_str(args, "channel_id", self.name())?;
        if crate::state::ChannelId::from_str(channel_id).is_none() {
            return Err(ImError::Parse(format!(
                "{}: 非 canonical channel_id",
                self.name()
            )));
        }
        let posts = args
            .get("posts")
            .and_then(Value::as_array)
            .filter(|items| !items.is_empty())
            .ok_or_else(|| {
                ImError::Parse(format!(
                    "{}: posts 必须是非空 canonical 字符串数组",
                    self.name()
                ))
            })?;
        let posts: Vec<&str> = posts
            .iter()
            .map(|post| {
                let post = post.as_str().ok_or_else(|| {
                    ImError::Parse(format!("{}: posts 只能包含字符串", self.name()))
                })?;
                if !crate::state::is_canonical_post_id(post) {
                    return Err(ImError::Parse(format!(
                        "{}: posts 只能包含 canonical post id",
                        self.name()
                    )));
                }
                Ok(post)
            })
            .collect::<Result<_, ImError>>()?;
        Ok((
            "post/read",
            json!({ "channelId": channel_id, "posts": posts }),
        ))
    }
}
static POST_READ: PostReadCommand = PostReadCommand;
inventory::submit! {
    OutboundRegistration {
        name: "im_post_read",
        command: &POST_READ,
    }
}

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
inventory::submit! {
    OutboundRegistration {
        name: "im_revoke",
        command: &REVOKE,
    }
}

/// #3 POST /api/cses/posts/createSchedule — 定时消息 → post_schedule_created。
/// 真源 CreateSchedulePostReq{Post *Post, ScheduledTime int64 `json:"schedulePostAt"`}。
/// Mobile canonical 的 `type`/`props` 属于 post 业务字段；`temporary_id` 由 Helix 发送
/// 状态机拥有，定时消息入口不得接收调用方伪造的临时 ID。
struct CreateScheduleCommand;
impl OutboundCommand for CreateScheduleCommand {
    fn name(&self) -> &'static str {
        "im_create_schedule"
    }
    /// 将冻结后的 G08 command 转为 Go wire，correlation 元字段不进入 body。
    fn build(&self, args: &Value) -> Result<(&'static str, Value), ImError> {
        require_exact_keys(
            args,
            &[
                "channel_id",
                "message",
                "schedule_post_at",
                "type",
                "props",
                "req_id",
            ],
            self.name(),
        )?;
        let channel_id = require_str(args, "channel_id", self.name())?;
        let message = require_str(args, "message", self.name())?;
        let schedule_post_at = args
            .get("schedule_post_at")
            .and_then(Value::as_i64)
            .filter(|value| *value > 0)
            .ok_or_else(|| {
                ImError::Parse(format!(
                    "{}: 缺/坏 schedule_post_at（正 int64 毫秒）",
                    self.name()
                ))
            })?;
        let mut post = json!({ "channelId": channel_id, "message": message });
        if let Some(post_type) = args.get("type") {
            let post_type = post_type
                .as_str()
                .filter(|value| !value.is_empty())
                .ok_or_else(|| ImError::Parse(format!("{}: type 必须为非空字符串", self.name())))?;
            post["type"] = json!(post_type);
        }
        if let Some(props) = args.get("props") {
            if !props.is_object() {
                return Err(ImError::Parse(format!(
                    "{}: props 必须为 object",
                    self.name()
                )));
            }
            post["props"] = props.clone();
        }
        Ok((
            "posts/createSchedule",
            json!({ "post": post, "schedulePostAt": schedule_post_at }),
        ))
    }
}
static CREATE_SCHEDULE: CreateScheduleCommand = CreateScheduleCommand;
inventory::submit! {
    OutboundRegistration {
        name: "im_create_schedule",
        command: &CREATE_SCHEDULE,
    }
}

/// 拒绝未冻结的 command 字段，避免平台偷偷扩张 G08/G09 业务协议。
fn require_exact_keys(args: &Value, allowed: &[&str], cmd: &str) -> Result<(), ImError> {
    let object = args
        .as_object()
        .ok_or_else(|| ImError::Parse(format!("{cmd}: payload 必须为 object")))?;
    if let Some(unknown) = object.keys().find(|key| !allowed.contains(&key.as_str())) {
        return Err(ImError::Parse(format!("{cmd}: 未知字段 {unknown}")));
    }
    Ok(())
}

/// #4 POST /api/cses/posts/cancelSchedule — 取消定时 → post_schedule_canceled。
/// 真源 CancelSchedulePostReq{ChannelId}；userId 取 session。
struct CancelScheduleCommand;
impl OutboundCommand for CancelScheduleCommand {
    fn name(&self) -> &'static str {
        "im_cancel_schedule"
    }
    /// 将冻结后的 G09 command 转为 Go wire，req_id 只留在 Helix correlation。
    fn build(&self, args: &Value) -> Result<(&'static str, Value), ImError> {
        require_exact_keys(args, &["channel_id", "req_id"], self.name())?;
        let channel_id = require_str(args, "channel_id", self.name())?;
        Ok(("posts/cancelSchedule", json!({ "channelId": channel_id })))
    }
}
static CANCEL_SCHEDULE: CancelScheduleCommand = CancelScheduleCommand;
inventory::submit! {
    OutboundRegistration {
        name: "im_cancel_schedule",
        command: &CANCEL_SCHEDULE,
    }
}
