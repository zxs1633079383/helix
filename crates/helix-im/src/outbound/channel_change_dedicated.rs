//! channel 写族补全 outbound 命令（HTTP-coverage 补齐，与 channel_change.rs 零交集）。
//!
//! channel_change.rs 已落地聚合 `change/info` + source/picture/notice/top/permission。本文件补
//! 现网客户端**实际单独调用**的专用写路由（Go 各有独立 handler，前端 message-v3 走专用端点而非
//! 聚合 info）：displayName / orient / purpose / add·remove/manger / enableApproval / view。
//!
//! ## endpoint / body 真源（mattermost csesapi 逐字，full-map partials/2，不自造 fixture，C5）
//! - displayName #13 `command.ChangeChannelDisPlayName`：`{id, displayName}`（**id**=channelId，
//!   字段名 DisPlayName 但 json key `displayName`）→ WS update_channel。
//! - orient #16 / purpose #17 共用 `command.UpdateChannelGuidePurposeCommand`：`{id, orient?, purpose?}`
//!   （**id**=channelId，两字段各自端点只填本字段）→ WS update_channel。
//! - add/manger #19 `command.AddChannelMangerCommand`：`{channelId, users:[{id,name,role,teamId}]}`
//!   → 当前 WS 已注释（仅 GrpcInvoke 对端，迁移注意点）。
//! - remove/manger #20 `command.DeleteChannelMangerCommand`：同 add 结构 → WS 已注释。
//! - enableApproval #3 `entity.EnableApprovalParam`：`{channelId, approvalStatus:bool}` → change_channel_approval。
//! - view #4 `entity.ViewChannels`：`{channels:[{id, isRoot:bool}]}` → 无 WS（HTTP 直返）。
//!
//! **casing/键名陷阱**：displayName/orient/purpose 用 **`id`** 作 channelId 键；add/remove/manger +
//! enableApproval 用 **`channelId`**；view 用 channels 数组每项 `id`。同域不同键 = Go decode 静默失败
//! 防线（写错即后端收下空 id 拒绝）。UserId/OwnerId 由后端 session 覆盖（json:"-"），core 不填。

use linkme::distributed_slice;
use serde_json::{json, Value};

use crate::error::ImError;

use super::registry::{require_str, OutboundCommand, OutboundRegistration, OUTBOUND_COMMANDS};

/// 取必填 `users` 数组（add/remove manger 用 `[{id,name,role,teamId}]`，非空、元素全 object）。
/// 边界零信任：缺/空/类型错 → Err（HX-C 不变量 4），不 panic。
fn require_member_array(args: &Value, cmd: &str) -> Result<Value, ImError> {
    args.get("users")
        .and_then(Value::as_array)
        .filter(|a| !a.is_empty() && a.iter().all(Value::is_object))
        .cloned()
        .map(Value::Array)
        .ok_or_else(|| ImError::Parse(format!("{cmd}: 缺/空 users（非空 object 数组）")))
}

// ── #13 channel/change/displayName — 改群展示名 → update_channel ───────────────────
struct ChangeDisplayNameCommand;
impl OutboundCommand for ChangeDisplayNameCommand {
    fn name(&self) -> &'static str {
        "im_channel_change_display_name"
    }
    fn build(&self, args: &Value) -> Result<(&'static str, Value), ImError> {
        let channel_id = require_str(args, "channel_id", self.name())?;
        // displayName 必填（改名语义需非空；Go ChangeChannelDisPlayName.DisPlayName string 非指针）。
        let display_name = require_str(args, "display_name", self.name())?;
        Ok((
            "channel/change/displayName",
            json!({ "id": channel_id, "displayName": display_name }),
        ))
    }
}
#[distributed_slice(OUTBOUND_COMMANDS)]
static CHANGE_DISPLAY_NAME_REG: OutboundRegistration = OutboundRegistration {
    name: "im_channel_change_display_name",
    command: &ChangeDisplayNameCommand,
};

// ── #16 channel/change/orient — 改群导向 → update_channel ──────────────────────────
struct ChangeOrientCommand;
impl OutboundCommand for ChangeOrientCommand {
    fn name(&self) -> &'static str {
        "im_channel_change_orient"
    }
    fn build(&self, args: &Value) -> Result<(&'static str, Value), ImError> {
        let channel_id = require_str(args, "channel_id", self.name())?;
        // orient 必填（Go UpdateChannelGuidePurposeCommand.Orient *string，本端点改导向语义需有值）。
        let orient = require_str(args, "orient", self.name())?;
        Ok((
            "channel/change/orient",
            json!({ "id": channel_id, "orient": orient }),
        ))
    }
}
#[distributed_slice(OUTBOUND_COMMANDS)]
static CHANGE_ORIENT_REG: OutboundRegistration = OutboundRegistration {
    name: "im_channel_change_orient",
    command: &ChangeOrientCommand,
};

// ── #17 channel/change/purpose — 改群简介 → update_channel ─────────────────────────
struct ChangePurposeCommand;
impl OutboundCommand for ChangePurposeCommand {
    fn name(&self) -> &'static str {
        "im_channel_change_purpose"
    }
    fn build(&self, args: &Value) -> Result<(&'static str, Value), ImError> {
        let channel_id = require_str(args, "channel_id", self.name())?;
        // purpose 必填（同 orient，本端点专改简介，允许空串需走聚合 info；此处改简介语义取非空）。
        let purpose = require_str(args, "purpose", self.name())?;
        Ok((
            "channel/change/purpose",
            json!({ "id": channel_id, "purpose": purpose }),
        ))
    }
}
#[distributed_slice(OUTBOUND_COMMANDS)]
static CHANGE_PURPOSE_REG: OutboundRegistration = OutboundRegistration {
    name: "im_channel_change_purpose",
    command: &ChangePurposeCommand,
};

// ── #19 channel/add/manger — 添加群管理员（WS 已注释，仅 GrpcInvoke）────────────────
struct AddMangerCommand;
impl OutboundCommand for AddMangerCommand {
    fn name(&self) -> &'static str {
        "im_channel_add_manger"
    }
    fn build(&self, args: &Value) -> Result<(&'static str, Value), ImError> {
        let channel_id = require_str(args, "channel_id", self.name())?;
        let users = require_member_array(args, self.name())?;
        Ok((
            "channel/add/manger",
            json!({ "channelId": channel_id, "users": users }),
        ))
    }
}
#[distributed_slice(OUTBOUND_COMMANDS)]
static ADD_MANGER_REG: OutboundRegistration = OutboundRegistration {
    name: "im_channel_add_manger",
    command: &AddMangerCommand,
};

// ── #20 channel/remove/manger — 移除群管理员（WS 已注释，仅 GrpcInvoke）──────────────
struct RemoveMangerCommand;
impl OutboundCommand for RemoveMangerCommand {
    fn name(&self) -> &'static str {
        "im_channel_remove_manger"
    }
    fn build(&self, args: &Value) -> Result<(&'static str, Value), ImError> {
        let channel_id = require_str(args, "channel_id", self.name())?;
        let users = require_member_array(args, self.name())?;
        Ok((
            "channel/remove/manger",
            json!({ "channelId": channel_id, "users": users }),
        ))
    }
}
#[distributed_slice(OUTBOUND_COMMANDS)]
static REMOVE_MANGER_REG: OutboundRegistration = OutboundRegistration {
    name: "im_channel_remove_manger",
    command: &RemoveMangerCommand,
};

// ── #3 channels/enableApproval — 开/关群审批 → change_channel_approval ──────────────
struct EnableApprovalCommand;
impl OutboundCommand for EnableApprovalCommand {
    fn name(&self) -> &'static str {
        "im_channel_enable_approval"
    }
    fn build(&self, args: &Value) -> Result<(&'static str, Value), ImError> {
        let channel_id = require_str(args, "channel_id", self.name())?;
        // approvalStatus 必填 bool（Go *bool，开/关语义需显式；handler 直接解引用未防 nil）。
        let approval_status = args
            .get("approval_status")
            .and_then(Value::as_bool)
            .ok_or_else(|| {
                ImError::Parse(format!("{}: 缺 approval_status（bool）", self.name()))
            })?;
        Ok((
            "channels/enableApproval",
            json!({ "channelId": channel_id, "approvalStatus": approval_status }),
        ))
    }
}
#[distributed_slice(OUTBOUND_COMMANDS)]
static ENABLE_APPROVAL_REG: OutboundRegistration = OutboundRegistration {
    name: "im_channel_enable_approval",
    command: &EnableApprovalCommand,
};

// ── #4 channels/view — 批量标记频道已查看（无 WS）──────────────────────────────────
struct ViewChannelsCommand;
impl OutboundCommand for ViewChannelsCommand {
    fn name(&self) -> &'static str {
        "im_channels_view"
    }
    fn build(&self, args: &Value) -> Result<(&'static str, Value), ImError> {
        // channels 必填非空数组（每项 {id, isRoot?}）。Go handler 对 id=="" 报错、isRoot nil 默认 true。
        // 透传前端已构造好的 channels（snake_case 入参 channels，元素键直接是 wire camelCase id/isRoot）。
        let channels = args
            .get("channels")
            .and_then(Value::as_array)
            .filter(|a| !a.is_empty() && a.iter().all(Value::is_object))
            .cloned()
            .map(Value::Array)
            .ok_or_else(|| {
                ImError::Parse(format!(
                    "{}: 缺/空 channels（非空 object 数组）",
                    self.name()
                ))
            })?;
        Ok(("channels/view", json!({ "channels": channels })))
    }
}
#[distributed_slice(OUTBOUND_COMMANDS)]
static VIEW_CHANNELS_REG: OutboundRegistration = OutboundRegistration {
    name: "im_channels_view",
    command: &ViewChannelsCommand,
};

#[cfg(test)]
#[path = "channel_change_dedicated_tests.rs"]
mod tests;
