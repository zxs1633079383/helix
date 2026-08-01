use std::borrow::Cow;

use bytes::Bytes;
use helix_core::tick::AppCommand;
use serde_json::{json, Map, Value};

use crate::error::ImError;

/// 公开命令名到内核命令名的显式映射。
pub struct CommandAlias {
    pub public: &'static str,
    pub core: &'static str,
}

pub const COMMAND_ALIASES: &[CommandAlias] = &[
    CommandAlias {
        public: "im_send",
        core: "im_send_message",
    },
    CommandAlias {
        public: "im_relay_messages",
        core: "im_create_posts",
    },
    CommandAlias {
        public: "im_read_channel",
        core: "im_channels_view",
    },
    CommandAlias {
        public: "im_sync_channels",
        core: "im_sync_channels",
    },
    CommandAlias {
        public: "im_ensure_channel_loaded",
        core: "im_channel_load_increment_by_channel_id",
    },
    CommandAlias {
        public: "im_locate_post",
        core: "im_get_posts",
    },
    CommandAlias {
        public: "im_channel_create",
        core: "im_create_channel",
    },
    CommandAlias {
        public: "im_member_leave",
        core: "im_channel_leave",
    },
    CommandAlias {
        public: "im_channel_settings",
        core: "im_channel_change_info",
    },
    CommandAlias {
        public: "im_post_pin",
        core: "im_set_message_top",
    },
    // MessageV3 Phase 1 Gate MV3-G07b 声明的权威公开命令名是 `im_save_announcement`
    // （specs/003-messagev3-gate-inventory/gates/announcement/mv3-g07b/inbound-sample.json）。
    // 内核命令名 `im_announcement_save` 保持不变，只在公开边界收敛命名漂移。
    CommandAlias {
        public: "im_save_announcement",
        core: "im_announcement_save",
    },
    CommandAlias {
        public: "im_announcement_write",
        core: "im_announcement_save",
    },
    // MessageV3 Phase 1 定时 / 草稿域：公开名与内核名同名，此处**显式登记**是为了让
    // `command_alias` / `capabilities_json` 把它们当成已知公开命令暴露给三端 driver，
    // 而不是靠 `unwrap_or(public)` 的隐式兜底（对齐同名登记先例 `im_sync_channels`）。
    // MV3-G04d：查询定时消息（`posts/getSchedule`，Effect 仅 Http，零事件）。
    CommandAlias {
        public: "im_get_schedule",
        core: "im_get_schedule",
    },
    // MV3-G04a / MV3-G04c：创建定时消息（`posts/createSchedule`）。
    CommandAlias {
        public: "im_create_schedule",
        core: "im_create_schedule",
    },
    // MV3-G04b / MV3-G04c / MV3-G04e：取消定时消息（`posts/cancelSchedule`）。
    // MV3-G04c 的「编辑」与 MV3-G04e 的「立即发送」都由调用方编排为**两条独立公开命令**
    // （cancel + create / send + cancel），Helix 不提供合并命令，Angular 也不合并状态。
    CommandAlias {
        public: "im_cancel_schedule",
        core: "im_cancel_schedule",
    },
    // MV3-G04e：立即发送复用发送内核命令；此处显式登记公开名，使 `im_send_message` 与
    // `im_cancel_schedule` 在同一张公开命令表上可被三端 driver 组合调用。
    CommandAlias {
        public: "im_send_message",
        core: "im_send_message",
    },
    // MV3-G02e：草稿保存 / 查询（Effect 仅 Persist，零事件，结构化 Command Result）。
    CommandAlias {
        public: "im_save_draft",
        core: "im_save_draft",
    },
    CommandAlias {
        public: "im_query_draft",
        core: "im_query_draft",
    },
    CommandAlias {
        public: "im_online_status",
        core: "im_channel_online_status",
    },
    CommandAlias {
        public: "im_members_by_ids",
        core: "im_channels_members_by_ids",
    },
    CommandAlias {
        public: "im_list_candidate_users",
        core: "im_user_candidates",
    },
    CommandAlias {
        public: "im_channel_admin_change",
        core: "im_channel_member_role",
    },
    CommandAlias {
        public: "im_member_nickname_change",
        core: "im_update_member_nickname",
    },
    CommandAlias {
        public: "im_todo_query",
        core: "im_query_todo_list",
    },
    CommandAlias {
        public: "im_system_notice_query",
        core: "im_load_target_notifications",
    },
    CommandAlias {
        public: "im_module_read",
        core: "im_get_all_modules",
    },
    CommandAlias {
        public: "im_module_read_all",
        core: "im_get_all_modules",
    },
    CommandAlias {
        public: "im_company_group_upsert",
        core: "im_team_upsert",
    },
    CommandAlias {
        public: "im_company_group_maintain",
        core: "im_team_member_add",
    },
    CommandAlias {
        public: "im_company_exit",
        core: "im_team_quit",
    },
];

pub fn command_alias(public: &str) -> Option<&'static str> {
    COMMAND_ALIASES
        .iter()
        .find(|alias| alias.public == public)
        .map(|alias| alias.core)
}

pub fn normalize_command_payload(public: &str, payload: &[u8]) -> Result<Vec<u8>, ImError> {
    let mut value: Value = serde_json::from_slice(payload)
        .map_err(|e| ImError::Parse(format!("{public} payload: {e}")))?;
    let obj = value
        .as_object_mut()
        .ok_or_else(|| ImError::Parse(format!("{public} payload must be object")))?;
    normalize_top_level_keys(obj);
    normalize_public_semantics(public, obj)?;
    serde_json::to_vec(&value).map_err(|e| ImError::Serialize(e.to_string()))
}

/// 把三端公开业务命令统一转换为 core 的 [`AppCommand`]。
///
/// alias、payload 归一化与静态命令名驻留只在此处发生；平台 driver 只负责把自己的
/// 字符串/字节边界转换成该函数的入参，并把错误映射为平台错误码。
pub fn build_command(public_name: &str, payload: &[u8]) -> Result<AppCommand, ImError> {
    if public_name == crate::module::RUNTIME_IDENTITY_COMMAND {
        return Err(ImError::Parse(
            "reserved runtime command is not part of the public command API".to_string(),
        ));
    }
    let normalized_payload = normalize_command_payload(public_name, payload)?;
    let core_name = command_alias(public_name).unwrap_or(public_name);
    let name = match crate::outbound::canonical_command_name(core_name) {
        Some(static_name) => Cow::Borrowed(static_name),
        None => Cow::Owned(core_name.to_string()),
    };
    Ok(AppCommand {
        name,
        payload: Bytes::from(normalized_payload),
    })
}

fn normalize_top_level_keys(obj: &mut Map<String, Value>) {
    let keys: Vec<String> = obj.keys().cloned().collect();
    for key in keys {
        let normalized = camel_to_snake(&key);
        if normalized != key {
            move_key(obj, &key, &normalized);
        }
    }
}

/// 在唯一公开边界把 UI 业务意图收敛为 Core builder 的冻结字段。
fn normalize_public_semantics(public: &str, obj: &mut Map<String, Value>) -> Result<(), ImError> {
    match public {
        "im_send" | "im_send_message" => {
            obj.remove("temporary_id");
            obj.remove("allocated_temporary_id");
        }
        "im_send_quick_reply" => move_key(obj, "reaction", "emoji"),
        "im_read_channel" => {
            let channel_id = obj
                .get("channel_id")
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| ImError::Parse("im_read_channel: 缺 channel_id".to_string()))?;
            obj.insert(
                "channels".to_string(),
                json!([{ "id": channel_id, "isRoot": true }]),
            );
        }
        "im_mark_read" => {
            if !obj.contains_key("posts") {
                if let Some(post_id) = obj.get("post_id").and_then(Value::as_str) {
                    obj.insert("posts".to_string(), json!([post_id]));
                }
            }
        }
        "im_post_read" => {
            if !obj.contains_key("posts") {
                if let Some(post_ids) = obj.remove("post_ids") {
                    let post_ids = post_ids.as_array().ok_or_else(|| {
                        ImError::Parse("im_post_read: post_ids 必须是数组".to_string())
                    })?;
                    if post_ids.is_empty()
                        || post_ids.iter().any(|post_id| {
                            post_id
                                .as_str()
                                .is_none_or(|post_id| post_id.trim().is_empty())
                        })
                    {
                        return Err(ImError::Parse(
                            "im_post_read: post_ids 必须是非空字符串数组".to_string(),
                        ));
                    }
                    obj.insert("posts".to_string(), Value::Array(post_ids.clone()));
                } else if let Some(post_id) = obj
                    .get("post_id")
                    .and_then(Value::as_str)
                    .filter(|post_id| !post_id.trim().is_empty())
                {
                    obj.insert("posts".to_string(), json!([post_id]));
                }
            }
        }
        "im_channels_view" => {
            if !obj.contains_key("channels") {
                let channel_ids = obj.remove("channel_ids").ok_or_else(|| {
                    ImError::Parse("im_channels_view: 缺 channel_ids".to_string())
                })?;
                let channel_ids = channel_ids.as_array().ok_or_else(|| {
                    ImError::Parse("im_channels_view: channel_ids 必须是数组".to_string())
                })?;
                let mut seen = std::collections::HashSet::with_capacity(channel_ids.len());
                let mut channels = Vec::with_capacity(channel_ids.len());
                for channel_id in channel_ids {
                    let channel_id = channel_id
                        .as_str()
                        .map(str::trim)
                        .filter(|channel_id| !channel_id.is_empty())
                        .ok_or_else(|| {
                            ImError::Parse(
                                "im_channels_view: channel_ids 必须是非空字符串数组".to_string(),
                            )
                        })?;
                    if seen.insert(channel_id.to_string()) {
                        channels.push(json!({"id": channel_id, "isRoot": true}));
                    }
                }
                if channels.is_empty() {
                    return Err(ImError::Parse(
                        "im_channels_view: channel_ids 不能为空".to_string(),
                    ));
                }
                obj.insert("channels".to_string(), Value::Array(channels));
            }
        }
        "im_get_replies" => move_key(obj, "post_id", "reply_id"),
        "im_get_reply_branch" => move_key(obj, "post_id", "reply_first_level_id"),
        "im_locate_post" => {
            if !obj.contains_key("post_ids") {
                if let Some(post_id) = obj.get("post_id").and_then(Value::as_str) {
                    obj.insert("post_ids".to_string(), json!([post_id]));
                }
            }
            obj.insert("locate".to_string(), Value::Bool(true));
        }
        // MV3-G04a / MV3-G04c：把 Angular 的定时创建意图摊平成内核冻结键集。
        //
        // Angular Port（`createSchedule`）与 Gate 参考样本
        // （`gates/schedule/mv3-g04a/inbound-sample.json`）提交的是嵌套意图
        // `{channel_id, execute_at|send_at, post:{type,text|message,props,…}}`；
        // 内核 builder `im_create_schedule` 的冻结键集是
        // `{channel_id, message, schedule_post_at, type, props, req_id}` 且 `forbid_unknown`。
        // 收敛只发生在这里：wire body 与冻结键集都不变，公开边界只做摊平 + 改名。
        //
        // `post` 摊平后必须**移除**，否则 `require_exact_keys` 会把它当未知字段整条拒掉
        // （表现为「定时消息点了没反应、也没有任何 HTTP」）。
        // `post.viewers` / `post.mentions` 随 `post` 一并丢弃：Go 的
        // `CreateSchedulePostReq.Post` 在本期冻结形态里不承载它们，摊平后再塞进顶层
        // 反而会被冻结键集拒绝。
        "im_create_schedule" => {
            if !obj.contains_key("schedule_post_at") {
                move_key(obj, "execute_at", "schedule_post_at");
            }
            if !obj.contains_key("schedule_post_at") {
                move_key(obj, "send_at", "schedule_post_at");
            }
            if let Some(post) = obj.remove("post").and_then(|post| match post {
                Value::Object(post) => Some(post),
                _ => None,
            }) {
                // 正文：Angular 用 `text`，Go wire 用 `message`；两者同源，取先有者。
                if !obj.contains_key("message") {
                    if let Some(message) = post.get("message").or_else(|| post.get("text")).cloned()
                    {
                        obj.insert("message".to_string(), message);
                    }
                }
                if !obj.contains_key("type") {
                    if let Some(post_type) = post.get("type").cloned() {
                        obj.insert("type".to_string(), post_type);
                    }
                }
                // props 是 MessageV3/Go 共享的 opaque 业务 JSON，整棵原样搬运不改键。
                if !obj.contains_key("props") {
                    if let Some(props) = post.get("props").cloned() {
                        obj.insert("props".to_string(), props);
                    }
                }
                // temporaryId 归 Helix 发送状态机。调用方若在 post 里塞了它，这里原样
                // 提到顶层，由内核 builder 的冻结键集 fail-closed 拒绝——**不能静默吞掉**，
                // 否则前端伪造临时 ID 会变成「看起来成功、对账键却是假的」。
                if !obj.contains_key("temporary_id") {
                    if let Some(temporary_id) = post
                        .get("temporaryId")
                        .or_else(|| post.get("temporary_id"))
                        .cloned()
                    {
                        obj.insert("temporary_id".to_string(), temporary_id);
                    }
                }
            }
        }
        "im_channel_settings" => {
            let settings = obj
                .remove("settings")
                .and_then(|value| value.as_object().cloned())
                .ok_or_else(|| ImError::Parse("im_channel_settings: 缺 settings".to_string()))?;
            copy_setting(
                &settings,
                obj,
                &["displayName", "display_name"],
                "display_name",
            );
            copy_setting(&settings, obj, &["description", "purpose"], "purpose");
            copy_setting(&settings, obj, &["rules", "header"], "header");
            copy_setting(
                &settings,
                obj,
                &["expectedVersion", "expected_version"],
                "expected_version",
            );
        }
        _ => {}
    }
    Ok(())
}

fn copy_setting(
    settings: &Map<String, Value>,
    target: &mut Map<String, Value>,
    keys: &[&str],
    target_key: &str,
) {
    if let Some(value) = keys.iter().find_map(|key| settings.get(*key)).cloned() {
        target.insert(target_key.to_string(), value);
    }
}

fn camel_to_snake(value: &str) -> String {
    let mut normalized = String::with_capacity(value.len());
    for ch in value.chars() {
        if ch.is_ascii_uppercase() {
            normalized.push('_');
            normalized.push(ch.to_ascii_lowercase());
        } else {
            normalized.push(ch);
        }
    }
    normalized
}

fn move_key(obj: &mut Map<String, Value>, from: &str, to: &str) {
    if !obj.contains_key(to) {
        if let Some(value) = obj.remove(from) {
            obj.insert(to.to_string(), value);
        }
    } else {
        obj.remove(from);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use helix_core::effect::Effect;
    use helix_core::Correlation;

    /// MessageV3 批量已读公开意图必须收敛成 outbound builder 唯一识别的 `posts`。
    #[test]
    fn post_read_public_intent_normalizes_post_ids_to_posts() {
        let normalized = normalize_command_payload(
            "im_post_read",
            br#"{"channelId":"ch1","postIds":["p1","p2"]}"#,
        )
        .expect("normalize post read");
        let value: Value = serde_json::from_slice(&normalized).expect("decode post read");

        assert_eq!(value["channel_id"], "ch1");
        assert_eq!(value["posts"], json!(["p1", "p2"]));
        assert!(value.get("post_ids").is_none());
    }

    /// MessageV3 会话已读公开意图必须把频道 ID 变为 Go `/channels/view` 的对象数组。
    #[test]
    fn channels_view_public_intent_normalizes_channel_ids_to_objects() {
        let normalized =
            normalize_command_payload("im_channels_view", br#"{"channelIds":["ch1","ch2"]}"#)
                .expect("normalize channels view");
        let value: Value = serde_json::from_slice(&normalized).expect("decode channels view");

        assert_eq!(
            value["channels"],
            json!([
                {"id":"ch1","isRoot":true},
                {"id":"ch2","isRoot":true}
            ])
        );
        assert!(value.get("channel_ids").is_none());
    }

    /// 从公开 postRead 意图贯穿到 Go `/post/read` body，防止字段在 builder 前静默丢失。
    #[test]
    fn post_read_public_intent_reaches_go_posts_wire() {
        let command = build_command(
            "im_post_read",
            br#"{"channelId":"ch1","postIds":["p1","p2"]}"#,
        )
        .expect("build post read command");
        let effects = crate::outbound::handle_outbound(
            command.name.as_ref(),
            command.payload.as_ref(),
            "http://im/api/cses",
            "http://default",
            Some("conn-1"),
            Correlation::from_raw(1),
        )
        .expect("dispatch post read");

        match &effects[0] {
            Effect::Http { req, .. } => {
                let body: Value =
                    serde_json::from_slice(req.body.as_ref().expect("post read body").as_ref())
                        .expect("decode post read body");
                assert_eq!(req.url, "http://im/api/cses/post/read");
                assert_eq!(body, json!({"channelId":"ch1","posts":["p1","p2"]}));
            }
            other => panic!("expected Http, got {other:?}"),
        }
    }

    /// 从公开 channelsView 意图贯穿到 Go `/channels/view` body，并冻结 `isRoot=true`。
    #[test]
    fn channels_view_public_intent_reaches_go_channels_wire() {
        let command = build_command("im_channels_view", br#"{"channelIds":["ch1","ch2"]}"#)
            .expect("build channels view command");
        let effects = crate::outbound::handle_outbound(
            command.name.as_ref(),
            command.payload.as_ref(),
            "http://im/api/cses",
            "http://default",
            Some("conn-1"),
            Correlation::from_raw(2),
        )
        .expect("dispatch channels view");

        match &effects[0] {
            Effect::Http { req, .. } => {
                let body: Value =
                    serde_json::from_slice(req.body.as_ref().expect("channels view body").as_ref())
                        .expect("decode channels view body");
                assert_eq!(req.url, "http://im/api/cses/channels/view");
                assert_eq!(
                    body,
                    json!({"channels":[
                        {"id":"ch1","isRoot":true},
                        {"id":"ch2","isRoot":true}
                    ]})
                );
            }
            other => panic!("expected Http, got {other:?}"),
        }
    }
}
