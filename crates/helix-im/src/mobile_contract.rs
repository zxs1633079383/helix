use serde_json::{json, Map, Value};

use crate::error::ImError;

pub const CONTRACT_VERSION: u32 = 1;
pub const BUS_CHANNEL: &str = "im:__bus__";

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
        core: "im_post_read",
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

pub const PUBLIC_EVENTS: &[&str] = &[
    "im:post:sending",
    "im:post:received",
    "im:post:updated",
    "im:post:read",
    "im:channel:read_echo",
    "im:post:deleted",
    "im:post:batch-updated",
    "im:messages:query_result",
    "im:messages:older_loaded",
    "im:channels:projection",
    "im:channels:loaded",
    "im:channel:increment",
    "im:channel:update",
    "im:channel:update-by-post",
    "im:channel:created",
    "im:channel:closed",
    "im:channel:schedule-created",
    "im:channel:schedule-canceled",
    "im:channel:member-updated",
    "im:channel:memberNickname",
    "im:channel:members",
    "im:channel:replies",
    "im:post:located",
    "im:read:result",
    "im:todo:updated",
    "im:health:result",
    "im:net:reconnect_requested",
    "im:http:unauthorized",
    "im:net:offline",
];

pub const META_QUERIES: &[&str] = &["helix_capabilities", "helix_version"];

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

pub fn capabilities_json() -> Vec<u8> {
    let mut commands = crate::outbound::outbound_command_names();
    commands.push("im_send");
    commands.push("im_retry_send");
    commands.push("im_reconnect");
    commands.sort_unstable();
    commands.dedup();

    let mut queries = META_QUERIES.to_vec();
    queries.extend_from_slice(crate::query::query_command_names());
    queries.sort_unstable();
    queries.dedup();

    let value = json!({
        "abi": CONTRACT_VERSION,
        "bus": BUS_CHANNEL,
        "commands": commands,
        "queries": queries,
        "events": PUBLIC_EVENTS,
        "features": {
            "pendingSendTimeoutUnsend": true,
            "batchEventCallback": true,
            "readResultReqId": true,
            "noPerBusinessAbiSymbol": true
        }
    });
    serde_json::to_vec(&value).expect("static capabilities JSON must serialize")
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
        _ => {}
    }
    Ok(())
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
    fn capabilities_do_not_advertise_send_failed_as_p0() {
        let caps: serde_json::Value = serde_json::from_slice(&capabilities_json()).unwrap();
        assert_eq!(caps["abi"], 1);
        assert_eq!(caps["bus"], "im:__bus__");
        assert_eq!(caps["features"]["pendingSendTimeoutUnsend"], true);
        let commands = caps["commands"].as_array().unwrap();
        assert!(commands.iter().any(|q| q == "im_retry_send"));
        let queries = caps["queries"].as_array().unwrap();
        assert!(queries.iter().any(|q| q == "helix_capabilities"));
        assert!(queries.iter().any(|q| q == "helix_version"));
        assert!(queries.iter().any(|q| q == "im_query_dialog_list"));
        assert!(queries.iter().any(|q| q == "im_query_messages_by_channel"));
        assert!(caps["features"].get("sendFailedProjection").is_none());
        let events = caps["events"].as_array().unwrap();
        assert!(events.iter().any(|e| e == "im:post:sending"));
        assert!(events.iter().any(|e| e == "im:post:received"));
        assert!(!events.iter().any(|e| e == "im:post:send-failed"));
    }

    #[test]
    fn outbound_command_names_is_publicly_reexported() {
        assert_eq!(
            crate::outbound::outbound_command_names(),
            crate::outbound::registry::outbound_command_names()
        );
    }
}
