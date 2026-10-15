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

use serde_json::{json, Value};

use crate::error::ImError;

use crate::outbound::registry::{require_str, OutboundCommand, OutboundRegistration};

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
inventory::submit! {
    OutboundRegistration {
        name: "im_channel_change_display_name",
        command: &ChangeDisplayNameCommand,
    }
}

// ── #16 channel/change/orient — 改群导向 → update_channel ──────────────────────────
struct ChangeOrientCommand;
impl OutboundCommand for ChangeOrientCommand {
    fn name(&self) -> &'static str {
        "im_channel_change_orient"
    }
    fn build(&self, args: &Value) -> Result<(&'static str, Value), ImError> {
        let channel_id = require_str(args, "channel_id", self.name())?;
        // orient 允许空串清空，但长度必须符合 Go 的 30 字符上限。
        let orient = args
            .get("orient")
            .and_then(Value::as_str)
            .ok_or_else(|| ImError::Parse(format!("{}: 缺 orient（string）", self.name())))?;
        if orient.chars().count() > 30 {
            return Err(ImError::Parse(format!(
                "{}: orient 最多 30 个字符",
                self.name()
            )));
        }
        Ok((
            "channel/change/orient",
            json!({ "id": channel_id, "orient": orient }),
        ))
    }
}
inventory::submit! {
    OutboundRegistration {
        name: "im_channel_change_orient",
        command: &ChangeOrientCommand,
    }
}

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
inventory::submit! {
    OutboundRegistration {
        name: "im_channel_change_purpose",
        command: &ChangePurposeCommand,
    }
}

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
inventory::submit! {
    OutboundRegistration {
        name: "im_channel_add_manger",
        command: &AddMangerCommand,
    }
}

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
inventory::submit! {
    OutboundRegistration {
        name: "im_channel_remove_manger",
        command: &RemoveMangerCommand,
    }
}

// ── UC-6.2 channel/add|remove/manger — 设/撤管理员（单命令·role + endpoint 路由下沉）──────
/// **body + endpoint 路由成形下沉 helix（薄壳合规）**：壳只传结构化 args `{channel_id, user_id,
/// team_id, set:bool}`；role 业务赋值（set=true→ADMIN / set=false→MEMBER）+ endpoint 路由
/// （set=true→`channel/add/manger` / set=false→`channel/remove/manger`）+ users 单成员定点拼装
/// 在此收口。users 四键 `{id,name,role,teamId}` 全 camelCase（id 定点键·name 仅展示留空·真源
/// §19/§20）。与既有 Add/RemoveMangerCommand（直供 users[] 透传）并存：本命令是壳实际走的路由，
/// 既有两条保留注册（无害·壳不再用）。
struct SetMangerCommand;
impl OutboundCommand for SetMangerCommand {
    fn name(&self) -> &'static str {
        "im_channel_set_manger"
    }
    fn build(&self, args: &Value) -> Result<(&'static str, Value), ImError> {
        let channel_id = require_str(args, "channel_id", self.name())?;
        let user_id = require_str(args, "user_id", self.name())?;
        let team_id = args.get("team_id").and_then(Value::as_str).unwrap_or("");
        let set = args
            .get("set")
            .and_then(Value::as_bool)
            .ok_or_else(|| ImError::Parse(format!("{}: 缺 set（bool）", self.name())))?;
        // role 随设/撤切换（设=ADMIN·撤=MEMBER·与 WS channel_member_role_updated echo data.role 对齐）。
        let role = if set { "ADMIN" } else { "MEMBER" };
        let endpoint = if set {
            "channel/add/manger"
        } else {
            "channel/remove/manger"
        };
        let users = json!([{ "id": user_id, "name": "", "role": role, "teamId": team_id }]);
        Ok((endpoint, json!({ "channelId": channel_id, "users": users })))
    }
}
inventory::submit! {
    OutboundRegistration {
        name: "im_channel_set_manger",
        command: &SetMangerCommand,
    }
}

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
inventory::submit! {
    OutboundRegistration {
        name: "im_channel_enable_approval",
        command: &EnableApprovalCommand,
    }
}

// ── #4 channels/view — 批量标记频道已查看（无 WS）──────────────────────────────────
struct ViewChannelsCommand;
impl OutboundCommand for ViewChannelsCommand {
    fn name(&self) -> &'static str {
        "im_channels_view"
    }

    /// `/channels/view` 是无 WS 回声的读命令，必须注册 PortReply 以回灌唯一 read result。
    fn is_read(&self) -> bool {
        true
    }

    fn build(&self, args: &Value) -> Result<(&'static str, Value), ImError> {
        // 只接收 client_api 已一次归一化的 `{id,isRoot:true,throughPostId?}`，这里不再补字段或改 id。
        let channels = require_canonical_root_channels(args, self.name())?;
        Ok(("channels/view", json!({ "channels": channels })))
    }
}

/// 验证 channels/view 的冻结 wire 结构，拒绝重复 id、非 root 和未知字段。
fn require_canonical_root_channels(args: &Value, command: &str) -> Result<Value, ImError> {
    let channels = args
        .get("channels")
        .and_then(Value::as_array)
        .filter(|items| !items.is_empty())
        .ok_or_else(|| {
            ImError::Parse(format!(
                "{command}: 缺/空 channels（非空 canonical object 数组）"
            ))
        })?;
    let mut seen = std::collections::HashSet::with_capacity(channels.len());
    for channel in channels {
        let object = channel
            .as_object()
            .ok_or_else(|| ImError::Parse(format!("{command}: channel 必须是 object")))?;
        if !object.contains_key("id")
            || !object.contains_key("isRoot")
            || object
                .keys()
                .any(|key| !["id", "isRoot", "throughPostId"].contains(&key.as_str()))
        {
            return Err(ImError::Parse(format!(
                "{command}: channel 只能包含 id/isRoot/throughPostId canonical 字段"
            )));
        }
        let id = object
            .get("id")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty() && id.trim() == *id)
            .ok_or_else(|| ImError::Parse(format!("{command}: id 必须是非空 canonical 字符串")))?;
        if crate::state::ChannelId::from_str(id).is_none() {
            return Err(ImError::Parse(format!(
                "{command}: 非 canonical channel id: {id}"
            )));
        }
        if object.get("isRoot").and_then(Value::as_bool) != Some(true) {
            return Err(ImError::Parse(format!("{command}: isRoot 必须为 true")));
        }
        if let Some(post) = object.get("throughPostId") {
            if post
                .as_str()
                .is_none_or(|id| !crate::state::is_canonical_post_id(id))
            {
                return Err(ImError::Parse(format!(
                    "{command}: throughPostId 必须是 canonical post id"
                )));
            }
        }
        if !seen.insert(id) {
            return Err(ImError::Parse(format!("{command}: channel id 重复: {id}")));
        }
    }
    Ok(Value::Array(channels.to_vec()))
}
inventory::submit! {
    OutboundRegistration {
        name: "im_channels_view",
        command: &ViewChannelsCommand,
    }
}

#[cfg(test)]
#[path = "change_dedicated_tests.rs"]
mod tests;
