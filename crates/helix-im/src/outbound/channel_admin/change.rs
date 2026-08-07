//! channel change 族 outbound 命令（P3b，parallelGroup=G1）：info / source / picture / notice / top。
//!
//! 五条群信息修改命令，各独立 endpoint + body + inventory 注册（C4 收口，与 P3a/其他
//! channel_*.rs 零交集）。除 #15 change/props（故意不广播，见文末注释）外，本族 4 条触发
//! `update_channel`；notice 复用 settings CAS authority，成功后也只走 `update_channel`。
//!
//! ## endpoint / body 真源（mattermost csesapi 逐字，不自造 fixture，C5）
//! 子路由前缀 `channel/change`（BaseRoutes.Channel = `/api/cses/channel`，api.go）。
//! - info   #11 `command.ChangeChannelInfo`：`{channelId, name?, displayName?, picture?, pictureType?,
//!   header?, purpose?, orient?, module?, source?}`，UserId 由后端 session 覆盖（json:"-" 不读 body）。
//! - source #12 `command.ChangeChannelSource`：`{id, title}`（**id**=channelId，非 channelId 键）。
//! - picture#14 解码到 `*ent.Channel`：读 `{id, pictureType, picture}`（id=channelId）。
//! - notice #10 `command.ChangeChannelNotice`：`{id, notice:{text}, expectedVersion}`（**id**=channelId）。
//! - top    #18 `command.UpdateChannelTopCommand`：`{channelId, top:bool}`（per-member 置顶）。
//!
//! **casing/键名陷阱**：source/picture/notice 用 **`id`** 作 channelId 键（command json tag `json:"id"`），
//! info/top 用 **`channelId`** —— 同族不同键，decode 静默失败防线（写错即 Go 收下空 id 拒绝）。

use serde_json::{json, Value};

use crate::error::ImError;

use crate::outbound::registry::{require_str, OutboundCommand, OutboundRegistration};

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
        if let Some(purpose) = args.get("purpose") {
            if !purpose.is_null() && !purpose.is_string() {
                return Err(ImError::Parse(format!(
                    "{}: purpose 必须为字符串",
                    self.name()
                )));
            }
        }
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
        } else if args.get("purpose").and_then(Value::as_str).is_some() {
            return Err(ImError::Parse(format!(
                "{}: purpose 修改必须携带 expected_version",
                self.name()
            )));
        }
        Ok(("channel/change/info", b))
    }
}
static CHANGE_INFO: ChangeInfoCommand = ChangeInfoCommand;
inventory::submit! {
    OutboundRegistration {
        name: "im_channel_change_info",
        command: &CHANGE_INFO,
    }
}

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
inventory::submit! {
    OutboundRegistration {
        name: "im_channel_change_source",
        command: &CHANGE_SOURCE,
    }
}

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
inventory::submit! {
    OutboundRegistration {
        name: "im_channel_change_picture",
        command: &CHANGE_PICTURE,
    }
}

/// #10 POST /api/cses/channel/change/notice — 公告 CAS → update_channel（不发旧 notice 事件）。
/// 真源 `command.ChangeChannelNotice`：`{id, notice:{text}, expectedVersion}`。
struct ChangeNoticeCommand;
impl OutboundCommand for ChangeNoticeCommand {
    fn name(&self) -> &'static str {
        "im_channel_change_notice"
    }
    fn build(&self, args: &Value) -> Result<(&'static str, Value), ImError> {
        let channel_id = require_str(args, "channel_id", self.name())?;
        let notice = args
            .get("notice")
            .filter(|value| value.is_object())
            .ok_or_else(|| ImError::Parse(format!("{}: 缺 notice 对象", self.name())))?;
        let text = notice
            .get("text")
            .and_then(Value::as_str)
            .ok_or_else(|| ImError::Parse(format!("{}: notice.text 必须为字符串", self.name())))?;
        let expected_version = args
            .get("expected_version")
            .and_then(Value::as_i64)
            .filter(|version| *version >= 0)
            .ok_or_else(|| {
                ImError::Parse(format!(
                    "{}: expected_version 必须为非负 int64",
                    self.name()
                ))
            })?;
        Ok((
            "channel/change/notice",
            json!({
                "id": channel_id,
                "notice": {"text": text},
                "expectedVersion": expected_version,
            }),
        ))
    }
}
static CHANGE_NOTICE: ChangeNoticeCommand = ChangeNoticeCommand;
inventory::submit! {
    OutboundRegistration {
        name: "im_channel_change_notice",
        command: &CHANGE_NOTICE,
    }
}

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
inventory::submit! {
    OutboundRegistration {
        name: "im_channel_change_top",
        command: &CHANGE_TOP,
    }
}

/// #?? POST /api/cses/channel/change/permission — 改群权限（@提醒/公告/置顶权限）→ update_channel。
/// body = `{channelId, mentionPermission?, noticePermission?, topPermission?}`。
/// 权限是 partial patch；每个显式值必须属于 Go authority 的四值 allowlist。
struct ChangePermissionCommand;
impl OutboundCommand for ChangePermissionCommand {
    fn name(&self) -> &'static str {
        "im_channel_change_permission"
    }

    /// 将权限 partial patch 序列化为无版本条件的 Go HTTP 请求。
    fn build(&self, args: &Value) -> Result<(&'static str, Value), ImError> {
        let channel_id = require_str(args, "channel_id", self.name())?;
        let mut body = json!({ "channelId": channel_id });
        let mut patch_count = 0;
        for (arg_key, wire_key) in [
            ("mention_permission", "mentionPermission"),
            ("notice_permission", "noticePermission"),
            ("top_permission", "topPermission"),
        ] {
            if let Some(value) = args.get(arg_key) {
                let permission = value
                    .as_str()
                    .filter(|value| matches!(*value, "BOSS" | "CREATOR" | "MANAGER" | "MEMBER"))
                    .ok_or_else(|| {
                        ImError::Parse(format!(
                            "{}: {arg_key} 必须是 BOSS/CREATOR/MANAGER/MEMBER",
                            self.name()
                        ))
                    })?;
                body[wire_key] = json!(permission);
                patch_count += 1;
            }
        }
        if patch_count == 0 {
            return Err(ImError::Parse(format!(
                "{}: 至少提供一个权限字段",
                self.name()
            )));
        }
        Ok(("channel/change/permission", body))
    }
}
static CHANGE_PERMISSION: ChangePermissionCommand = ChangePermissionCommand;
inventory::submit! {
    OutboundRegistration {
        name: "im_channel_change_permission",
        command: &CHANGE_PERMISSION,
    }
}

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

    /// im_channel_change_permission 正常路径：partial patch 且不携带旧版本条件字段。
    #[test]
    fn change_permission_dispatch_and_body() {
        assert!(is_outbound("im_channel_change_permission"));
        let corr = Correlation::from_raw(1);
        let payload = serde_json::to_vec(&json!({
            "channel_id": "c1",
            "mention_permission": "CREATOR",
            "top_permission": "MEMBER",
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
                assert_eq!(body["mentionPermission"], "CREATOR");
                assert!(body.get("noticePermission").is_none());
                assert_eq!(body["topPermission"], "MEMBER");
                let legacy_version_key = format!("{}{}", "expected", "Version");
                assert!(body.get(&legacy_version_key).is_none());
            }
            other => panic!("expected Http, got {other:?}"),
        }
    }

    /// 缺全部权限或使用未知枚举 → Err（边界零信任，不 panic）。
    #[test]
    fn change_permission_missing_field_errors() {
        let corr = Correlation::from_raw(2);
        let payload = serde_json::to_vec(&json!({
            "channel_id": "c1",
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

        for invalid in [
            json!({"channel_id": "c1", "mention_permission": "OWNER"}),
        ] {
            assert!(handle_outbound(
                "im_channel_change_permission",
                &serde_json::to_vec(&invalid).unwrap(),
                "http://h/api",
                "http://h",
                None,
                corr,
            )
            .is_err());
        }
    }

    /// 公告命令只允许 `{text}` notice 与非负 expected_version，旧 map 形状一律拒绝。
    #[test]
    fn change_notice_requires_text_and_expected_version() {
        let corr = Correlation::from_raw(3);
        let payload = serde_json::to_vec(&json!({
            "channel_id": "c1",
            "notice": {"text": "公告"},
            "expected_version": 7,
        }))
        .unwrap();
        let effects = handle_outbound(
            "im_channel_change_notice",
            &payload,
            "http://h/api",
            "http://h",
            None,
            corr,
        )
        .expect("notice CAS should dispatch");
        let Effect::Http { req, .. } = &effects[0] else {
            panic!("expected HTTP effect")
        };
        let body: serde_json::Value = serde_json::from_slice(req.body.as_ref().unwrap()).unwrap();
        assert_eq!(
            body,
            json!({
                "id": "c1",
                "notice": {"text": "公告"},
                "expectedVersion": 7,
            })
        );

        for invalid in [
            json!({"channel_id": "c1", "notice": {"message": "公告"}, "expected_version": 7}),
            json!({"channel_id": "c1", "notice": {"text": "公告"}}),
            json!({"channel_id": "c1", "notice": {"text": "公告"}, "expected_version": -1}),
        ] {
            assert!(
                handle_outbound(
                    "im_channel_change_notice",
                    serde_json::to_vec(&invalid).unwrap().as_slice(),
                    "http://h/api",
                    "http://h",
                    None,
                    corr,
                )
                .is_err(),
                "invalid notice intent unexpectedly dispatched: {invalid}"
            );
        }
    }
}
