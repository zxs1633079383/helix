//! 既有 channel 域 outbound 命令（P2 registry 化迁移：leave / create / makeTopic /
//! member_change / close / nickname）。
//!
//! ⚠️ 本文件是 P2 迁移既有命令的临时归集；P3b 会补 channel 写族完整命令——届时按 C4 收口，
//! 各命令应裂回独立注册文件，本文件随之拆解。endpoint/body 真源逐条标注。

use linkme::distributed_slice;
use serde_json::{json, Value};

use crate::error::ImError;

use super::registry::{require_str, OutboundCommand, OutboundRegistration, OUTBOUND_COMMANDS};

/// #33 POST /api/cses/channel/member/leave — 退群（仅自己）→ channel_member_update + post。
/// 真源 entity.ChannelIdStruct{ChannelId}；UserId nil → session（Go 只允许自己退）。
struct ChannelLeaveCommand;
impl OutboundCommand for ChannelLeaveCommand {
    fn name(&self) -> &'static str {
        "im_channel_leave"
    }
    fn build(&self, args: &Value) -> Result<(&'static str, Value), ImError> {
        let channel_id = require_str(args, "channel_id", self.name())?;
        Ok(("channel/member/leave", json!({ "channelId": channel_id })))
    }
}
static CHANNEL_LEAVE: ChannelLeaveCommand = ChannelLeaveCommand;
#[distributed_slice(OUTBOUND_COMMANDS)]
static CHANNEL_LEAVE_REG: OutboundRegistration = OutboundRegistration {
    name: "im_channel_leave",
    command: &CHANNEL_LEAVE,
};

/// #7 POST /api/cses/channel/create — 建群 → channel_created + channel_member_update + post。
/// 真源 CreateChannelSpecifyOwner（内嵌 *ent.Channel + Users[]ChannelMemberMinimal）字段多，
/// 由前端直供 raw body（args 即 Go body 直传，helix-im 只认领 endpoint）。
struct CreateChannelCommand;
impl OutboundCommand for CreateChannelCommand {
    fn name(&self) -> &'static str {
        "im_create_channel"
    }
    fn build(&self, args: &Value) -> Result<(&'static str, Value), ImError> {
        Ok(("channel/create", args.clone()))
    }
}
static CREATE_CHANNEL: CreateChannelCommand = CreateChannelCommand;
#[distributed_slice(OUTBOUND_COMMANDS)]
static CREATE_CHANNEL_REG: OutboundRegistration = OutboundRegistration {
    name: "im_create_channel",
    command: &CREATE_CHANNEL,
};

/// #17 POST /api/cses/posts/makeTopic — 消息转话题 → channel_created + post_update。
/// 真源 {PostId, 内嵌 CreateChannelSpecifyOwner}；前端直供 raw body（须含 postId）。
struct MakeTopicCommand;
impl OutboundCommand for MakeTopicCommand {
    fn name(&self) -> &'static str {
        "im_make_topic"
    }
    fn build(&self, args: &Value) -> Result<(&'static str, Value), ImError> {
        // 至少校验 postId 存在（边界零信任），其余字段直传。
        require_str(args, "postId", self.name())?;
        Ok(("posts/makeTopic", args.clone()))
    }
}
static MAKE_TOPIC: MakeTopicCommand = MakeTopicCommand;
#[distributed_slice(OUTBOUND_COMMANDS)]
static MAKE_TOPIC_REG: OutboundRegistration = OutboundRegistration {
    name: "im_make_topic",
    command: &MAKE_TOPIC,
};

/// POST /api/cses/channel/member/change — 加/退成员 → channel_member_update。
/// 真源（真机curl真源 §5）：{channelId, joinUsers?:[{id,teamId,role}], leaveUsers?:[...]}，
/// joinUsers/leaveUsers 两者可同时非空。成员对象结构较复杂，由前端直供 raw body；至少校验
/// channelId 存在。注意：与 #33 `channel/member/leave`（自己退群）是两条——本条 admin 增减成员。
struct MemberChangeCommand;
impl OutboundCommand for MemberChangeCommand {
    fn name(&self) -> &'static str {
        "im_channel_member_change"
    }
    fn build(&self, args: &Value) -> Result<(&'static str, Value), ImError> {
        require_str(args, "channelId", self.name())?;
        Ok(("channel/member/change", args.clone()))
    }
}
static MEMBER_CHANGE: MemberChangeCommand = MemberChangeCommand;
#[distributed_slice(OUTBOUND_COMMANDS)]
static MEMBER_CHANGE_REG: OutboundRegistration = OutboundRegistration {
    name: "im_channel_member_change",
    command: &MEMBER_CHANGE,
};

/// #27 POST /api/cses/channel/close — 关闭频道 → channel_close（broadcast 到 channelId·自己也收）。
/// 真源 {ChannelId}。
struct ChannelCloseCommand;
impl OutboundCommand for ChannelCloseCommand {
    fn name(&self) -> &'static str {
        "im_channel_close"
    }
    fn build(&self, args: &Value) -> Result<(&'static str, Value), ImError> {
        let channel_id = require_str(args, "channel_id", self.name())?;
        Ok(("channel/close", json!({ "channelId": channel_id })))
    }
}
static CHANNEL_CLOSE: ChannelCloseCommand = ChannelCloseCommand;
#[distributed_slice(OUTBOUND_COMMANDS)]
static CHANNEL_CLOSE_REG: OutboundRegistration = OutboundRegistration {
    name: "im_channel_close",
    command: &CHANNEL_CLOSE,
};

/// #31 POST /api/cses/channel/member/change/nickname — 改群昵称 → update_channel_member_nickName
/// （现网 WS 名 camelCase `nickName`，broadcast 到 channelId）。
/// 真源 {ChannelId, UserId(nil→session), Nickname(trim 空→清空)}。
struct UpdateNicknameCommand;
impl OutboundCommand for UpdateNicknameCommand {
    fn name(&self) -> &'static str {
        "im_update_member_nickname"
    }
    fn build(&self, args: &Value) -> Result<(&'static str, Value), ImError> {
        let channel_id = require_str(args, "channel_id", self.name())?;
        // nickname 允许空（trim 空→Go 侧清空昵称），故不 require 非空。
        let nickname = args.get("nickname").and_then(Value::as_str).unwrap_or("");
        let mut b = json!({ "channelId": channel_id, "nickname": nickname });
        if let Some(uid) = args.get("user_id").and_then(Value::as_str) {
            b["userId"] = json!(uid);
        }
        Ok(("channel/member/change/nickname", b))
    }
}
static UPDATE_NICKNAME: UpdateNicknameCommand = UpdateNicknameCommand;
#[distributed_slice(OUTBOUND_COMMANDS)]
static UPDATE_NICKNAME_REG: OutboundRegistration = OutboundRegistration {
    name: "im_update_member_nickname",
    command: &UPDATE_NICKNAME,
};
