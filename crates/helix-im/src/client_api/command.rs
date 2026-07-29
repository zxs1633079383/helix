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
    CommandAlias {
        public: "im_announcement_write",
        core: "im_announcement_save",
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
        "im_create_schedule" => {
            if !obj.contains_key("schedule_post_at") {
                move_key(obj, "send_at", "schedule_post_at");
            }
            if let Some(post) = obj.get("post").and_then(Value::as_object).cloned() {
                if !obj.contains_key("message") {
                    if let Some(message) = post.get("message").cloned() {
                        obj.insert("message".to_string(), message);
                    }
                }
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
