//! channel 成员管理 outbound 命令（P3b，parallelGroup=G1）：member role / notify。
//!
//! 成员 change/leave/nickname 已在 `channel_existing.rs`（P2 迁移：im_channel_member_change /
//! im_channel_leave / im_update_member_nickname）；本文件补 P3b 漏项 role(#29) + notify(#30)。
//! 各独立 endpoint + body + inventory 注册（C4 收口，零交集）。
//!
//! ## endpoint / body 真源（mattermost csesapi 逐字，不自造 fixture，C5）
//! 子路由前缀 `channel/member/change`（BaseRoutes.ChannelMember = `/api/cses/channel/member`）。
//! - role  #29 `command.UpdateChannelMemberRole`：`{channelId, userIds:[]string, role}`
//!   → WS `channel_member_role_updated`（broadcast 到 channelId）。
//! - notify#30 `command.UpdateChannelNotify`：`{channelId, notify}`，UserId 后端 session 覆盖
//!   → WS `update_channel`（**定向本人**）。
//!
//! **键名陷阱（仅注释标注，不在本 Phase outbound 断言）**：notify 的 **request body** 用
//! `channelId`（command json tag）；其 WS **响应** payload 经 `UpdateChannelNotifyVo.ToMap()`
//! 序列化时 key 是 **`id`** 而非 channelId（#30 迁移注意点 3）——那是 P4 WS 事件解析侧的事，
//! P3b 只产 outbound request，**不**把 `id` 键混进 request body。

use serde_json::{json, Value};

use crate::error::ImError;

use crate::outbound::registry::{require_str, OutboundCommand, OutboundRegistration};

/// #29 POST /api/cses/channel/member/change/role — 改成员角色 → channel_member_role_updated。
/// 真源 `{channelId, userIds:[]string(非空), role}`，OwnerId 后端 session 覆盖。
struct MemberRoleCommand;
impl OutboundCommand for MemberRoleCommand {
    fn name(&self) -> &'static str {
        "im_channel_member_role"
    }
    fn build(&self, args: &Value) -> Result<(&'static str, Value), ImError> {
        let channel_id = require_str(args, "channel_id", self.name())?;
        // userIds 非空字符串数组（Go Validate：len(UserIds) > 0）。
        let user_ids = args
            .get("user_ids")
            .and_then(Value::as_array)
            .filter(|a| !a.is_empty() && a.iter().all(Value::is_string))
            .cloned()
            .map(Value::Array)
            .ok_or_else(|| {
                ImError::Parse(format!("{}: 缺/空 user_ids（非空字符串数组）", self.name()))
            })?;
        let role = require_str(args, "role", self.name())?;
        if !matches!(role, "ADMIN" | "MEMBER") {
            return Err(ImError::Parse(format!(
                "{}: role 必须是 ADMIN/MEMBER",
                self.name()
            )));
        }
        Ok((
            "channel/member/change/role",
            json!({ "channelId": channel_id, "userIds": user_ids, "role": role }),
        ))
    }
}
static MEMBER_ROLE: MemberRoleCommand = MemberRoleCommand;
inventory::submit! {
    OutboundRegistration {
        name: "im_channel_member_role",
        command: &MEMBER_ROLE,
    }
}

/// #30 POST /api/cses/channel/member/change/notify — 改成员免打扰 → update_channel（定向本人）。
/// 真源 `command.UpdateChannelNotify`：`{channelId, notify}`，UserId(json:"-") 后端 session 覆盖。
struct MemberNotifyCommand;
impl OutboundCommand for MemberNotifyCommand {
    fn name(&self) -> &'static str {
        "im_channel_member_notify"
    }
    // 仅接受 Go NotifyType 的三个稳定字符串，避免非法枚举透传到 HTTP/持久化边界。
    fn build(&self, args: &Value) -> Result<(&'static str, Value), ImError> {
        let channel_id = require_str(args, "channel_id", self.name())?;
        // notify 必填且严格属于 NORMAL/STRONG/IGNORE（Go NotifyType.Values()）。
        let notify = args
            .get("notify")
            .and_then(Value::as_str)
            .filter(|value| matches!(*value, "NORMAL" | "STRONG" | "IGNORE"))
            .ok_or_else(|| {
                ImError::Parse(format!(
                    "{}: notify 必须是 NORMAL/STRONG/IGNORE",
                    self.name()
                ))
            })?;
        Ok((
            "channel/member/change/notify",
            json!({ "channelId": channel_id, "notify": notify }),
        ))
    }
}
static MEMBER_NOTIFY: MemberNotifyCommand = MemberNotifyCommand;
inventory::submit! {
    OutboundRegistration {
        name: "im_channel_member_notify",
        command: &MEMBER_NOTIFY,
    }
}
