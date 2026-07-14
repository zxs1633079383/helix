use std::borrow::Cow;

use bytes::Bytes;
use helix_core::tick::AppCommand;
use serde_json::{json, Map, Value};

use crate::error::ImError;

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
        core: "im_channel_load_increment_by_channel_id",
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_command_is_the_single_alias_and_payload_normalization_path() {
        let command = build_command(
            "im_send",
            br#"{"channelId":"ch1","temporaryId":"tmp1","text":"hi","type":"TEXT"}"#,
        )
        .expect("build shared command");

        assert_eq!(command.name, "im_send_message");
        assert!(matches!(command.name, Cow::Owned(_)));
        let payload: Value = serde_json::from_slice(&command.payload).expect("decode payload");
        assert_eq!(payload["channel_id"], "ch1");
        assert_eq!(payload["temporary_id"], "tmp1");
        assert!(payload.get("channelId").is_none());
    }

    #[test]
    fn build_command_borrows_registered_names_and_owns_fallback_names() {
        let registered =
            build_command("im_revoke", br#"{"postId":"p1"}"#).expect("build registered command");
        assert!(matches!(registered.name, Cow::Borrowed("im_revoke")));

        let fallback = build_command("im_reconnect", br#"{}"#).expect("build fallback command");
        assert!(matches!(fallback.name, Cow::Owned(ref name) if name == "im_reconnect"));

        let query = build_command("im_query_dialog_list", br#"{}"#).expect("build query command");
        assert!(matches!(query.name, Cow::Owned(ref name) if name == "im_query_dialog_list"));
    }

    #[test]
    fn build_command_applies_public_semantics_before_aliasing() {
        let command = build_command("im_read_channel", br#"{"channelId":"ch1"}"#)
            .expect("build read-channel command");

        assert_eq!(command.name, "im_channels_view");
        let payload: Value = serde_json::from_slice(&command.payload).expect("decode payload");
        assert_eq!(payload["channels"], json!([{"id":"ch1","isRoot":true}]));
    }

    #[test]
    fn build_command_rejects_malformed_or_non_object_payloads() {
        assert!(matches!(
            build_command("im_revoke", b"{not-json"),
            Err(ImError::Parse(_))
        ));
        assert!(matches!(
            build_command("im_revoke", br#"[]"#),
            Err(ImError::Parse(_))
        ));
    }

    #[test]
    fn build_command_keeps_unknown_names_as_owned_compatibility_fallback() {
        let command = build_command("future_command", br#"{}"#)
            .expect("unknown commands remain forward-compatible");

        assert!(matches!(command.name, Cow::Owned(ref name) if name == "future_command"));
        assert_eq!(command.payload.as_ref(), br#"{}"#);
    }

    #[test]
    fn mobile_contract_compatibility_path_delegates_to_shared_builder() {
        let payload = br#"{"channelId":"ch1","postId":"p1"}"#;
        let current = build_command("im_revoke", payload).expect("build current command");
        let compatibility = crate::mobile_contract::build_command("im_revoke", payload)
            .expect("build compatibility command");

        assert_eq!(compatibility.name, current.name);
        assert_eq!(compatibility.payload, current.payload);
        assert_eq!(
            crate::mobile_contract::command_alias("im_send"),
            command_alias("im_send")
        );
    }

    #[test]
    fn send_alias_normalizes_to_existing_core_command_and_snake_payload() {
        let input = br#"{"channelId":"ch1","temporaryId":"tmp1","text":"hi","type":"TEXT"}"#;
        let normalized = normalize_command_payload("im_send", input).expect("normalize");
        let v: serde_json::Value = serde_json::from_slice(&normalized).unwrap();
        assert_eq!(command_alias("im_send"), Some("im_send_message"));
        assert_eq!(v["channel_id"], "ch1");
        assert_eq!(v["temporary_id"], "tmp1");
        assert_eq!(v["text"], "hi");
        assert_eq!(v["type"], "TEXT");
        assert!(v.get("channelId").is_none());
        assert!(v.get("temporaryId").is_none());
    }

    #[test]
    fn public_commands_normalize_only_top_level_keys_and_keep_props_opaque() {
        let input = br#"{"channelId":"ch1","postId":"p1","props":{"temporaryId":"opaque"}}"#;
        let normalized = normalize_command_payload("im_revoke", input).expect("normalize");
        let v: serde_json::Value = serde_json::from_slice(&normalized).unwrap();
        assert_eq!(v["channel_id"], "ch1");
        assert_eq!(v["post_id"], "p1");
        assert_eq!(v["props"]["temporaryId"], "opaque");
    }

    #[test]
    fn aliases_candidate_directory_without_requiring_ui_wire_fields() {
        let normalized = normalize_command_payload("im_list_candidate_users", br#"{}"#).unwrap();
        let value: Value = serde_json::from_slice(&normalized).unwrap();
        assert!(value.as_object().is_some_and(Map::is_empty));
        assert_eq!(
            command_alias("im_list_candidate_users"),
            Some("im_user_candidates")
        );
    }

    #[test]
    fn public_command_semantics_adapt_ui_intents_inside_helix() {
        let quick =
            normalize_command_payload("im_send_quick_reply", br#"{"postId":"p1","reaction":"ok"}"#)
                .unwrap();
        let quick: Value = serde_json::from_slice(&quick).unwrap();
        assert_eq!(quick["post_id"], "p1");
        assert_eq!(quick["emoji"], "ok");

        let read = normalize_command_payload("im_mark_read", br#"{"postId":"p1"}"#).unwrap();
        let read: Value = serde_json::from_slice(&read).unwrap();
        assert_eq!(read["posts"], json!(["p1"]));

        let channel_read =
            normalize_command_payload("im_read_channel", br#"{"channelId":"ch1"}"#).unwrap();
        let channel_read: Value = serde_json::from_slice(&channel_read).unwrap();
        assert_eq!(command_alias("im_read_channel"), Some("im_channels_view"));
        assert_eq!(
            channel_read["channels"],
            json!([{"id":"ch1","isRoot":true}])
        );

        let replies = normalize_command_payload("im_get_replies", br#"{"postId":"p1"}"#).unwrap();
        let replies: Value = serde_json::from_slice(&replies).unwrap();
        assert_eq!(replies["reply_id"], "p1");

        let branch =
            normalize_command_payload("im_get_reply_branch", br#"{"postId":"p2"}"#).unwrap();
        let branch: Value = serde_json::from_slice(&branch).unwrap();
        assert_eq!(branch["reply_first_level_id"], "p2");

        let locate = normalize_command_payload("im_locate_post", br#"{"postId":"p3"}"#).unwrap();
        let locate: Value = serde_json::from_slice(&locate).unwrap();
        assert_eq!(locate["post_ids"], json!(["p3"]));
        assert_eq!(locate["locate"], true);
    }

    #[test]
    fn channel_settings_intent_maps_render_fields_to_go_channel_fields() {
        let normalized = normalize_command_payload(
            "im_channel_settings",
            r#"{"channelId":"ch1","settings":{"displayName":"新群名","description":"群说明","rules":"群规则","expectedVersion":7}}"#.as_bytes(),
        )
        .expect("normalize settings");
        let value: Value = serde_json::from_slice(&normalized).expect("decode settings");
        assert_eq!(
            command_alias("im_channel_settings"),
            Some("im_channel_change_info")
        );
        assert_eq!(value["channel_id"], "ch1");
        assert_eq!(value["display_name"], "新群名");
        assert_eq!(value["purpose"], "群说明");
        assert_eq!(value["header"], "群规则");
        assert_eq!(value["expected_version"], 7);
        assert!(value.get("settings").is_none());
    }

    #[test]
    fn member_directory_alias_reuses_render_ready_by_ids_projection() {
        let normalized =
            normalize_command_payload("im_members_by_ids", br#"{"channelIds":["ch1","ch2"]}"#)
                .expect("normalize member directory");
        let value: Value = serde_json::from_slice(&normalized).expect("decode members request");
        assert_eq!(
            command_alias("im_members_by_ids"),
            Some("im_channels_members_by_ids")
        );
        assert_eq!(value["channel_ids"], json!(["ch1", "ch2"]));
    }

    #[test]
    fn schedule_intent_is_shaped_for_the_core_builder_without_touching_post_props() {
        let normalized = normalize_command_payload(
            "im_create_schedule",
            br#"{"channelId":"ch1","post":{"message":"later","temporaryId":"tmp1","props":{"sendAt":7}},"sendAt":1700000000000}"#,
        )
        .unwrap();
        let value: Value = serde_json::from_slice(&normalized).unwrap();
        assert_eq!(value["channel_id"], "ch1");
        assert_eq!(value["message"], "later");
        assert_eq!(value["temporary_id"], "tmp1");
        assert_eq!(value["schedule_post_at"], 1_700_000_000_000_i64);
        assert_eq!(value["post"]["props"]["sendAt"], 7);
    }

    #[test]
    fn outbound_command_names_is_publicly_reexported() {
        assert_eq!(
            crate::outbound::outbound_command_names(),
            crate::outbound::registry::outbound_command_names()
        );
    }
}
