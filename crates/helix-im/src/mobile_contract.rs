use serde_json::{json, Map, Value};

use crate::error::ImError;

pub const CONTRACT_VERSION: u32 = 1;
pub const BUS_CHANNEL: &str = "im:__bus__";

pub struct CommandAlias {
    pub public: &'static str,
    pub core: &'static str,
}

pub const COMMAND_ALIASES: &[CommandAlias] = &[CommandAlias {
    public: "im_send",
    core: "im_send_message",
}];

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
    match public {
        "im_send" => normalize_im_send_payload(payload),
        _ => Ok(payload.to_vec()),
    }
}

pub fn capabilities_json() -> Vec<u8> {
    let mut commands = crate::outbound::outbound_command_names();
    commands.push("im_send");
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

fn normalize_im_send_payload(payload: &[u8]) -> Result<Vec<u8>, ImError> {
    let mut value: Value = serde_json::from_slice(payload)
        .map_err(|e| ImError::Parse(format!("im_send payload: {e}")))?;
    let obj = value
        .as_object_mut()
        .ok_or_else(|| ImError::Parse("im_send payload must be object".into()))?;
    move_key(obj, "channelId", "channel_id");
    move_key(obj, "temporaryId", "temporary_id");
    serde_json::to_vec(&value).map_err(|e| ImError::Serialize(e.to_string()))
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
    fn capabilities_do_not_advertise_send_failed_as_p0() {
        let caps: serde_json::Value = serde_json::from_slice(&capabilities_json()).unwrap();
        assert_eq!(caps["abi"], 1);
        assert_eq!(caps["bus"], "im:__bus__");
        assert_eq!(caps["features"]["pendingSendTimeoutUnsend"], true);
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
