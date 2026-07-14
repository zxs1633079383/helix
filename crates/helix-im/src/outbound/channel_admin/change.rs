//! channel change 族 outbound 命令（P3b，parallelGroup=G1）：info / source / picture / notice / top。
//!
//! 五条群信息修改命令，各独立 endpoint + body + distributed_slice 注册（C4 收口，与 P3a/其他
//! channel_*.rs 零交集）。除 #15 change/props（故意不广播，见文末注释）外，本族 4 条触发
//! `update_channel`、notice 触发 `update_channel_notice`（WS 推送侧属 P4，本 Phase 只产 outbound）。
//!
//! ## endpoint / body 真源（mattermost csesapi 逐字，不自造 fixture，C5）
//! 子路由前缀 `channel/change`（BaseRoutes.Channel = `/api/cses/channel`，api.go）。
//! - info   #11 `command.ChangeChannelInfo`：`{channelId, name?, displayName?, picture?, pictureType?,
//!   header?, purpose?, orient?, module?, source?}`，UserId 由后端 session 覆盖（json:"-" 不读 body）。
//! - source #12 `command.ChangeChannelSource`：`{id, title}`（**id**=channelId，非 channelId 键）。
//! - picture#14 解码到 `*ent.Channel`：读 `{id, pictureType, picture}`（id=channelId）。
//! - notice #10 `command.ChangeChannelNotice`：`{id, notice}`（**id**=channelId，notice=map）。
//! - top    #18 `command.UpdateChannelTopCommand`：`{channelId, top:bool}`（per-member 置顶）。
//!
//! **casing/键名陷阱**：source/picture/notice 用 **`id`** 作 channelId 键（command json tag `json:"id"`），
//! info/top 用 **`channelId`** —— 同族不同键，decode 静默失败防线（写错即 Go 收下空 id 拒绝）。

use linkme::distributed_slice;
use serde_json::{json, Value};

use crate::error::ImError;

use crate::outbound::registry::{
    require_str, OutboundCommand, OutboundRegistration, OUTBOUND_COMMANDS,
};

/// 把可选字符串字段按 camelCase 透传进 body（仅 Some 时写入，对齐 Go `omitempty` 指针语义）。
fn put_opt_str(body: &mut Value, args: &Value, src_key: &str, wire_key: &str) {
    if let Some(v) = args.get(src_key).and_then(Value::as_str) {
        body[wire_key] = json!(v);
    }
}

/// 把可选 JSON 值（map/object，如 picture/source/notice）原样透传（仅存在时写入）。
fn put_opt_val(body: &mut Value, args: &Value, src_key: &str, wire_key: &str) {
    if let Some(v) = args.get(src_key) {
        if !v.is_null() {
            body[wire_key] = v.clone();
        }
    }
}

/// #11 POST /api/cses/channel/change/info — 聚合改名/头像/简介/导向/模块/来源 → update_channel。
/// 真源 `command.ChangeChannelInfo`（channelId 必填，其余字段可选 omitempty）。
struct ChangeInfoCommand;
impl OutboundCommand for ChangeInfoCommand {
    fn name(&self) -> &'static str {
        "im_channel_change_info"
    }
    fn build(&self, args: &Value) -> Result<(&'static str, Value), ImError> {
        let channel_id = require_str(args, "channel_id", self.name())?;
        let mut b = json!({ "channelId": channel_id });
        // 可选字段：仅显式提供时透传（Go 指针 omitempty，缺则保留既有）。
        put_opt_str(&mut b, args, "name", "name");
        put_opt_str(&mut b, args, "display_name", "displayName");
        put_opt_str(&mut b, args, "header", "header");
        put_opt_str(&mut b, args, "purpose", "purpose");
        put_opt_str(&mut b, args, "orient", "orient");
        put_opt_str(&mut b, args, "module", "module");
        put_opt_str(&mut b, args, "picture_type", "pictureType");
        put_opt_val(&mut b, args, "picture", "picture");
        put_opt_val(&mut b, args, "source", "source");
        if let Some(value) = args.get("expected_version") {
            let expected_version =
                value
                    .as_i64()
                    .filter(|version| *version >= 0)
                    .ok_or_else(|| {
                        ImError::Parse(format!(
                            "{}: expected_version 必须为非负 int64",
                            self.name()
                        ))
                    })?;
            b["expectedVersion"] = json!(expected_version);
        }
        Ok(("channel/change/info", b))
    }
}
static CHANGE_INFO: ChangeInfoCommand = ChangeInfoCommand;
#[distributed_slice(OUTBOUND_COMMANDS)]
static CHANGE_INFO_REG: OutboundRegistration = OutboundRegistration {
    name: "im_channel_change_info",
    command: &CHANGE_INFO,
};

/// #12 POST /api/cses/channel/change/source — 改频道来源标题 → update_channel。
/// 真源 `command.ChangeChannelSource`：`{id, title}`（**id**=channelId）。
struct ChangeSourceCommand;
impl OutboundCommand for ChangeSourceCommand {
    fn name(&self) -> &'static str {
        "im_channel_change_source"
    }
    fn build(&self, args: &Value) -> Result<(&'static str, Value), ImError> {
        let channel_id = require_str(args, "channel_id", self.name())?;
        // title 允许空串（Go ChangeChannelSource.Title string 非指针，不强制非空）。
        let title = args.get("title").and_then(Value::as_str).unwrap_or("");
        Ok((
            "channel/change/source",
            json!({ "id": channel_id, "title": title }),
        ))
    }
}
static CHANGE_SOURCE: ChangeSourceCommand = ChangeSourceCommand;
#[distributed_slice(OUTBOUND_COMMANDS)]
static CHANGE_SOURCE_REG: OutboundRegistration = OutboundRegistration {
    name: "im_channel_change_source",
    command: &CHANGE_SOURCE,
};

/// #14 POST /api/cses/channel/change/picture — 改群头像 → update_channel。
/// 真源解码到 `*ent.Channel`，读 `{id, pictureType, picture}`（Go 校验 pictureType/picture 非 nil）。
struct ChangePictureCommand;
impl OutboundCommand for ChangePictureCommand {
    fn name(&self) -> &'static str {
        "im_channel_change_picture"
    }
    fn build(&self, args: &Value) -> Result<(&'static str, Value), ImError> {
        let channel_id = require_str(args, "channel_id", self.name())?;
        let picture_type = require_str(args, "picture_type", self.name())?;
        // picture 是 map（ent.Channel.Picture map[string]interface{}）必填非 null（Go 校验非 nil）。
        let picture = args
            .get("picture")
            .filter(|v| !v.is_null())
            .ok_or_else(|| ImError::Parse(format!("{}: 缺 picture（map 非 null）", self.name())))?
            .clone();
        Ok((
            "channel/change/picture",
            json!({ "id": channel_id, "pictureType": picture_type, "picture": picture }),
        ))
    }
}
static CHANGE_PICTURE: ChangePictureCommand = ChangePictureCommand;
#[distributed_slice(OUTBOUND_COMMANDS)]
static CHANGE_PICTURE_REG: OutboundRegistration = OutboundRegistration {
    name: "im_channel_change_picture",
    command: &CHANGE_PICTURE,
};

/// #10 POST /api/cses/channel/change/notice — 改群公告 → update_channel_notice（可伴 post 系统消息）。
/// 真源 `command.ChangeChannelNotice`：`{id, notice}`（**id**=channelId，notice=map 指针）。
struct ChangeNoticeCommand;
impl OutboundCommand for ChangeNoticeCommand {
    fn name(&self) -> &'static str {
        "im_channel_change_notice"
    }
    fn build(&self, args: &Value) -> Result<(&'static str, Value), ImError> {
        let channel_id = require_str(args, "channel_id", self.name())?;
        // notice 是 map（*map[string]any）必填非 null（公告内容）。
        let notice = args
            .get("notice")
            .filter(|v| !v.is_null())
            .ok_or_else(|| ImError::Parse(format!("{}: 缺 notice（map 非 null）", self.name())))?
            .clone();
        Ok((
            "channel/change/notice",
            json!({ "id": channel_id, "notice": notice }),
        ))
    }
}
static CHANGE_NOTICE: ChangeNoticeCommand = ChangeNoticeCommand;
#[distributed_slice(OUTBOUND_COMMANDS)]
static CHANGE_NOTICE_REG: OutboundRegistration = OutboundRegistration {
    name: "im_channel_change_notice",
    command: &CHANGE_NOTICE,
};

/// #18 POST /api/cses/channel/change/top — per-member 置顶频道 → update_channel（定向本人）。
/// 真源 `command.UpdateChannelTopCommand`：`{channelId, top:bool}`。
struct ChangeTopCommand;
impl OutboundCommand for ChangeTopCommand {
    fn name(&self) -> &'static str {
        "im_channel_change_top"
    }
    fn build(&self, args: &Value) -> Result<(&'static str, Value), ImError> {
        let channel_id = require_str(args, "channel_id", self.name())?;
        // top 必填 bool（Go *bool，置顶/取消置顶语义需显式）。
        let top = args
            .get("top")
            .and_then(Value::as_bool)
            .ok_or_else(|| ImError::Parse(format!("{}: 缺 top（bool）", self.name())))?;
        Ok((
            "channel/change/top",
            json!({ "channelId": channel_id, "top": top }),
        ))
    }
}
static CHANGE_TOP: ChangeTopCommand = ChangeTopCommand;
#[distributed_slice(OUTBOUND_COMMANDS)]
static CHANGE_TOP_REG: OutboundRegistration = OutboundRegistration {
    name: "im_channel_change_top",
    command: &CHANGE_TOP,
};

/// #?? POST /api/cses/channel/change/permission — 改群权限（@提醒/公告/置顶权限）→ update_channel。
/// body = `{channelId, mentionPermission, noticePermission, topPermission}`（均 string，passthrough camelCase）。
/// 三权限字段必填非空（边界零信任，HX-C 不变量 4）。
struct ChangePermissionCommand;
impl OutboundCommand for ChangePermissionCommand {
    fn name(&self) -> &'static str {
        "im_channel_change_permission"
    }
    fn build(&self, args: &Value) -> Result<(&'static str, Value), ImError> {
        let channel_id = require_str(args, "channel_id", self.name())?;
        let mention_permission = require_str(args, "mention_permission", self.name())?;
        let notice_permission = require_str(args, "notice_permission", self.name())?;
        let top_permission = require_str(args, "top_permission", self.name())?;
        Ok((
            "channel/change/permission",
            json!({
                "channelId": channel_id,
                "mentionPermission": mention_permission,
                "noticePermission": notice_permission,
                "topPermission": top_permission,
            }),
        ))
    }
}
static CHANGE_PERMISSION: ChangePermissionCommand = ChangePermissionCommand;
#[distributed_slice(OUTBOUND_COMMANDS)]
static CHANGE_PERMISSION_REG: OutboundRegistration = OutboundRegistration {
    name: "im_channel_change_permission",
    command: &CHANGE_PERMISSION,
};

// #15 change/props（UpdateCsesChannelProps）**故意不实现 outbound 命令 + 故意不补 WS handler**：
// 真源 cses_channel.go:1226-1250 落库 Props 后**无 a.Publish**（与其他 change/* 不一致，迁移注意点 2）。
// 前端若需改 props 经通用 store 路径，不走 IM WS 对账——此处不新增 im_update_channel_props，
// 也**勿**为它补 update_channel WS handler（会凭空广播一个服务端不推的事件，破坏对账）。

#[cfg(test)]
mod tests {
    use helix_core::effect::Effect;
    use helix_core::Correlation;
    use serde_json::json;

    use crate::outbound::registry::{handle_outbound, is_outbound};

    /// im_channel_change_permission 正常路径：endpoint = channel/change/permission，
    /// body 四字段 camelCase 透传（channelId/mentionPermission/noticePermission/topPermission）。
    #[test]
    fn change_permission_dispatch_and_body() {
        assert!(is_outbound("im_channel_change_permission"));
        let corr = Correlation::from_raw(1);
        let payload = serde_json::to_vec(&json!({
            "channel_id": "c1",
            "mention_permission": "all",
            "notice_permission": "owner",
            "top_permission": "admin",
        }))
        .unwrap();
        let effects = handle_outbound(
            "im_channel_change_permission",
            &payload,
            "http://h/api",
            "http://h",
            Some("conn1"),
            corr,
        )
        .expect("change_permission should dispatch");
        match &effects[0] {
            Effect::Http { req, .. } => {
                assert!(
                    req.url.ends_with("/channel/change/permission"),
                    "url={}",
                    req.url
                );
                let body: serde_json::Value =
                    serde_json::from_slice(req.body.as_ref().expect("body")).unwrap();
                assert_eq!(body["channelId"], "c1");
                assert_eq!(body["mentionPermission"], "all");
                assert_eq!(body["noticePermission"], "owner");
                assert_eq!(body["topPermission"], "admin");
            }
            other => panic!("expected Http, got {other:?}"),
        }
    }

    /// 缺任一必填权限字段 → Err（边界零信任，不 panic）。
    #[test]
    fn change_permission_missing_field_errors() {
        let corr = Correlation::from_raw(2);
        let payload = serde_json::to_vec(&json!({
            "channel_id": "c1",
            "mention_permission": "all",
        }))
        .unwrap();
        assert!(handle_outbound(
            "im_channel_change_permission",
            &payload,
            "http://h/api",
            "http://h",
            None,
            corr,
        )
        .is_err());
    }
}
