//! 既有 channel 域 outbound 命令（P2 registry 化迁移：leave / create / makeTopic /
//! member_change / close / nickname）。
//!
//! ⚠️ 本文件是 P2 迁移既有命令的临时归集；P3b 会补 channel 写族完整命令——届时按 C4 收口，
//! 各命令应裂回独立注册文件，本文件随之拆解。endpoint/body 真源逐条标注。

use linkme::distributed_slice;
use serde_json::{json, Value};

use crate::error::ImError;

use crate::outbound::registry::{
    build_creator_member_users, require_str, OutboundCommand, OutboundRegistration,
    OUTBOUND_COMMANDS,
};

/// #33 POST /api/cses/channel/member/leave — 退群（仅自己）→ channel_member_update + post。
/// 真源 entity.ChannelIdStruct{ChannelId, UserId *string, NewCreator *ChannelMemberMinimal}
/// （full-map #33，channel.go:609-613，三键全 camelCase json tag）。
/// UC-5.3：群主退群仍是「自己退」(session.UserId==owner)，但需 NewCreator 指定继任群主——
/// 若静默丢 newCreator 则群主转让失效。userId/newCreator 为 Go *指针（omitempty 语义），
/// 故仅在前端供给时透传，不发空键。ChannelMemberMinimal 结构由前端直供，build 不解构透传整对象。
struct ChannelLeaveCommand;
impl OutboundCommand for ChannelLeaveCommand {
    fn name(&self) -> &'static str {
        "im_channel_leave"
    }
    fn build(&self, args: &Value) -> Result<(&'static str, Value), ImError> {
        let channel_id = require_str(args, "channel_id", self.name())?;
        let mut b = json!({ "channelId": channel_id });
        // userId *string：缺省 → Go 退 session 自己；显式给则透传（命名陷阱：userId camelCase）。
        if let Some(uid) = args.get("user_id").and_then(Value::as_str) {
            b["userId"] = json!(uid);
        }
        // newCreator *ChannelMemberMinimal：群主退群转让继任群主，整对象透传，build 不解构。
        if let Some(nc) = args.get("new_creator") {
            if !nc.is_null() {
                b["newCreator"] = nc.clone();
            }
        }
        Ok(("channel/member/leave", b))
    }
}
static CHANNEL_LEAVE: ChannelLeaveCommand = ChannelLeaveCommand;
#[distributed_slice(OUTBOUND_COMMANDS)]
static CHANNEL_LEAVE_REG: OutboundRegistration = OutboundRegistration {
    name: "im_channel_leave",
    command: &CHANNEL_LEAVE,
};

/// #7 POST /api/cses/channel/create — 建群 → channel_created + channel_member_update + post。
/// 真源 CreateChannelSpecifyOwner（内嵌 *ent.Channel + Users[]ChannelMemberMinimal）。
///
/// **body 成形下沉 helix（薄壳合规）**：壳只传结构化 args `{display_name, team_id, self_id,
/// member_ids:[]}`（身份/列表，非 wire body）；wire body + role 业务赋值（self CREATOR + 他人
/// MEMBER）在此拼成，对齐真机curl真源 §4：`{teamId, displayName, orient:"", type:"P",
/// users:[{id,teamId,role}], picturetype:"USER", picture:{userIds:[...]}, forceCreate:true}`。
struct CreateChannelCommand;
impl OutboundCommand for CreateChannelCommand {
    fn name(&self) -> &'static str {
        "im_create_channel"
    }
    fn build(&self, args: &Value) -> Result<(&'static str, Value), ImError> {
        let display_name = require_str(args, "display_name", self.name())?;
        let team_id = args.get("team_id").and_then(Value::as_str).unwrap_or("");
        let (users, user_ids) = build_creator_member_users(args, self.name())?;
        Ok((
            "channel/create",
            json!({
                "teamId": team_id,
                "displayName": display_name,
                "orient": "",
                "type": "P",
                "users": users,
                "picturetype": "USER",
                "picture": { "userIds": user_ids },
                "forceCreate": true,
            }),
        ))
    }
}
static CREATE_CHANNEL: CreateChannelCommand = CreateChannelCommand;
#[distributed_slice(OUTBOUND_COMMANDS)]
static CREATE_CHANNEL_REG: OutboundRegistration = OutboundRegistration {
    name: "im_create_channel",
    command: &CREATE_CHANNEL,
};

/// #17 POST /api/cses/posts/makeTopic — 消息转话题 → channel_created + post_update。
/// 真源 {PostId, 内嵌 CreateChannelSpecifyOwner}。
///
/// **body 成形下沉 helix（薄壳合规）**：壳只传结构化 args `{root_id, post_id, display_name,
/// team_id, self_id, member_ids:[]}`；wire body + role 业务赋值在此拼成，对齐真机curl真源 §2：
/// `{rootId, teamId, postId, displayName, type:"T", users:[{id,teamId,role}], picturetype:"USER",
/// picture:{userIds:[...]}, forceCreate:true}`。与建群（§4）同形态，差异 endpoint + rootId/postId
/// + type=T（话题）+ 无 orient。
struct MakeTopicCommand;
impl OutboundCommand for MakeTopicCommand {
    fn name(&self) -> &'static str {
        "im_make_topic"
    }
    fn build(&self, args: &Value) -> Result<(&'static str, Value), ImError> {
        let root_id = require_str(args, "root_id", self.name())?;
        let post_id = require_str(args, "post_id", self.name())?;
        let display_name = require_str(args, "display_name", self.name())?;
        let team_id = args.get("team_id").and_then(Value::as_str).unwrap_or("");
        let (users, user_ids) = build_creator_member_users(args, self.name())?;
        Ok((
            "posts/makeTopic",
            json!({
                "rootId": root_id,
                "teamId": team_id,
                "postId": post_id,
                "displayName": display_name,
                "type": "T",
                "users": users,
                "picturetype": "USER",
                "picture": { "userIds": user_ids },
                "forceCreate": true,
            }),
        ))
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
/// joinUsers/leaveUsers 两者可同时非空。注意：与 #33 `channel/member/leave`（自己退群）是两条——
/// 本条 admin 增减成员。
///
/// **body 成形下沉 helix（薄壳合规）**：壳只传结构化 args `{channel_id, team_id, self_id,
/// join_user_ids:[], leave_user_ids:[]}`；joinUsers/leaveUsers 数组 + role=MEMBER 业务赋值在此
/// 拼成。join 排除自身（拉别人进群·自身已在）；空集对应字段省略（不发空数组·与 joinUsers/leaveUsers
/// 可同时非 nil 语义对齐）。
struct MemberChangeCommand;
impl OutboundCommand for MemberChangeCommand {
    fn name(&self) -> &'static str {
        "im_channel_member_change"
    }
    fn build(&self, args: &Value) -> Result<(&'static str, Value), ImError> {
        let channel_id = require_str(args, "channel_id", self.name())?;
        let team_id = args.get("team_id").and_then(Value::as_str).unwrap_or("");
        let self_id = args.get("self_id").and_then(Value::as_str).unwrap_or("");
        // 拼成员对象数组（三键 id/teamId/role 全 camelCase·role=MEMBER）。过滤空 id；join 额外排除自身。
        let build_users = |key: &str, exclude_self: bool| -> Vec<Value> {
            args.get(key)
                .and_then(Value::as_array)
                .map(|arr| {
                    arr.iter()
                        .filter_map(Value::as_str)
                        .filter(|uid| !(uid.trim().is_empty() || exclude_self && *uid == self_id))
                        .map(|uid| json!({ "id": uid, "teamId": team_id, "role": "MEMBER" }))
                        .collect()
                })
                .unwrap_or_default()
        };
        let mut body = serde_json::Map::new();
        body.insert("channelId".into(), Value::String(channel_id.to_string()));
        let joins = build_users("join_user_ids", true);
        if !joins.is_empty() {
            body.insert("joinUsers".into(), Value::Array(joins));
        }
        let leaves = build_users("leave_user_ids", false);
        if !leaves.is_empty() {
            body.insert("leaveUsers".into(), Value::Array(leaves));
        }
        Ok(("channel/member/change", Value::Object(body)))
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
