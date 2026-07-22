//! 既有 channel 域 outbound 命令（P2 registry 化迁移：leave / create / makeTopic /
//! member_change / close / nickname）。
//!
//! ⚠️ 本文件是 P2 迁移既有命令的临时归集；P3b 会补 channel 写族完整命令——届时按 C4 收口，
//! 各命令应裂回独立注册文件，本文件随之拆解。endpoint/body 真源逐条标注。

use serde_json::{json, Value};

use crate::error::ImError;

use crate::outbound::registry::{require_str, OutboundCommand, OutboundRegistration};

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
inventory::submit! {
    OutboundRegistration {
        name: "im_channel_leave",
        command: &CHANNEL_LEAVE,
    }
}

/// G-15a POST /api/cses/channel/create — Helix 只做 canonical command 到 Go boundary 的 casing
/// adapter。会话身份、team、成员角色与默认 USER 头像均由 Go 根据 session 派生，不能由跨端内核
/// 生成或信任；自定义 PICTURE 头像只可走独立的改头像命令。可选 `req_id` 仅用于生成
/// `Cses-Track-Id` transport header，不进入 Go mutation body。
struct CreateChannelCommand;
impl OutboundCommand for CreateChannelCommand {
    fn name(&self) -> &'static str {
        "im_create_channel"
    }
    fn build(&self, args: &Value) -> Result<(&'static str, Value), ImError> {
        require_exact_keys(
            args,
            &["type", "user_ids", "display_name", "force_create", "req_id"],
            self.name(),
        )?;
        let channel_type = require_str(args, "type", self.name())?;
        let user_ids = require_non_empty_string_array(args, "user_ids", self.name())?;
        let display_name = require_str(args, "display_name", self.name())?;
        let force_create = require_bool(args, "force_create", self.name())?;
        if args.get("req_id").is_some() {
            require_str(args, "req_id", self.name())?;
        }
        Ok((
            "channel/create",
            json!({
                "type": channel_type,
                "userIds": user_ids,
                "displayName": display_name,
                "forceCreate": force_create,
            }),
        ))
    }
}
static CREATE_CHANNEL: CreateChannelCommand = CreateChannelCommand;
inventory::submit! {
    OutboundRegistration {
        name: "im_create_channel",
        command: &CREATE_CHANNEL,
    }
}

/// G-15 POST /api/cses/posts/makeTopic — 只透传 root post 与目标用户。source channel、
/// display name、team、creator role 和默认 USER 图片均由 Go 用 root post + session 派生；`req_id`
/// 只作为 host/Helix 关联键，不能泄漏进 HTTP body。
struct MakeTopicCommand;
impl OutboundCommand for MakeTopicCommand {
    fn name(&self) -> &'static str {
        "im_make_topic"
    }
    fn build(&self, args: &Value) -> Result<(&'static str, Value), ImError> {
        require_exact_keys(args, &["root_id", "user_ids", "req_id"], self.name())?;
        let root_id = require_str(args, "root_id", self.name())?;
        let user_ids = require_non_empty_string_array(args, "user_ids", self.name())?;
        require_str(args, "req_id", self.name())?;
        Ok((
            "posts/makeTopic",
            json!({
                "rootId": root_id,
                "userIds": user_ids,
            }),
        ))
    }
}
static MAKE_TOPIC: MakeTopicCommand = MakeTopicCommand;
inventory::submit! {
    OutboundRegistration {
        name: "im_make_topic",
        command: &MAKE_TOPIC,
    }
}

fn require_exact_keys(args: &Value, allowed: &[&str], cmd: &str) -> Result<(), ImError> {
    let object = args
        .as_object()
        .ok_or_else(|| ImError::Parse(format!("{cmd}: payload 必须是 object")))?;
    if let Some(unknown) = object.keys().find(|key| !allowed.contains(&key.as_str())) {
        return Err(ImError::Parse(format!(
            "{cmd}: 未知或非 canonical 字段 '{unknown}'"
        )));
    }
    Ok(())
}

fn require_non_empty_string_array(args: &Value, key: &str, cmd: &str) -> Result<Value, ImError> {
    let values = args
        .get(key)
        .and_then(Value::as_array)
        .filter(|values| !values.is_empty())
        .ok_or_else(|| ImError::Parse(format!("{cmd}: 缺/空必填数组字段 '{key}'")))?;
    if values
        .iter()
        .any(|value| value.as_str().is_none_or(|value| value.trim().is_empty()))
    {
        return Err(ImError::Parse(format!(
            "{cmd}: 字段 '{key}' 必须是非空字符串数组"
        )));
    }
    Ok(Value::Array(values.clone()))
}

fn require_bool(args: &Value, key: &str, cmd: &str) -> Result<bool, ImError> {
    args.get(key)
        .and_then(Value::as_bool)
        .ok_or_else(|| ImError::Parse(format!("{cmd}: 缺/类型错误的必填布尔字段 '{key}'")))
}

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
inventory::submit! {
    OutboundRegistration {
        name: "im_channel_member_change",
        command: &MEMBER_CHANGE,
    }
}

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
inventory::submit! {
    OutboundRegistration {
        name: "im_channel_close",
        command: &CHANNEL_CLOSE,
    }
}

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
inventory::submit! {
    OutboundRegistration {
        name: "im_update_member_nickname",
        command: &UPDATE_NICKNAME,
    }
}
