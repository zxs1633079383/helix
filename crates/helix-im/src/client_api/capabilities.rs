use serde_json::json;

use crate::{EVENT_HTTP_UNAUTHORIZED, EVENT_NET_OFFLINE};

pub const CONTRACT_VERSION: u32 = 1;
pub const BUS_CHANNEL: &str = "im:__bus__";

pub const PUBLIC_EVENTS: &[&str] = &[
    "im:post:sending",
    "im:post:received",
    "im:post:updated",
    "im:posts:batch-result",
    "im:post:read",
    "im:post:readers",
    "im:channel:read_echo",
    "im:post:deleted",
    "im:post:batch-updated",
    "im:post:send-failed",
    "im:post:increment-failed",
    "im:messages:query_result",
    "im:messages:older_loaded",
    "im:interaction:frame",
    "im:interaction:action-receipt",
    "im:channels:loaded",
    "im:channel:increment",
    "im:channel:update",
    "im:sync:state",
    "im:channel:created",
    "im:channel:closed",
    "im:channel:schedule-created",
    "im:channel:schedule-canceled",
    "im:channel:member-updated",
    "im:channel:settings-updated",
    "im:channel:left",
    "im:channel:memberNickname",
    "im:channel:members",
    "im:channel:replies",
    "im:post:located",
    "im:user:candidates",
    "im:read:result",
    "im:todo:updated",
    "im:health:result",
    "im:net:reconnect_requested",
    EVENT_HTTP_UNAUTHORIZED,
    EVENT_NET_OFFLINE,
];

pub const META_QUERIES: &[&str] = &["helix_capabilities", "helix_version"];

pub fn capabilities_json() -> Vec<u8> {
    let mut commands = crate::outbound::outbound_command_names();
    commands.push("im_send_message");
    commands.push("im_retry_send");
    commands.push("im_reconnect");
    commands.extend_from_slice(crate::query::query_command_names());
    commands.sort_unstable();
    commands.dedup();

    let queries = META_QUERIES;

    let value = json!({
        "abi": CONTRACT_VERSION,
        "bus": BUS_CHANNEL,
        "commands": commands,
        "queries": queries,
        "events": PUBLIC_EVENTS,
        "features": {
            "pendingSendTimeoutUnsend": true,
            "sendFailedProjection": true,
            "helixOwnedTemporaryId": true,
            "batchEventCallback": true,
            "readResultReqId": true,
            "leftDialogListFrameV1": true,
            "opaqueActionInvokeV1": true,
            "noPerBusinessAbiSymbol": true
        }
    });
    serde_json::to_vec(&value).expect("static capabilities JSON must serialize")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capabilities_advertise_canonical_send_and_runtime_failure_projections() {
        let caps: serde_json::Value = serde_json::from_slice(&capabilities_json()).unwrap();
        assert_eq!(caps["abi"], 1);
        assert_eq!(caps["bus"], "im:__bus__");
        assert_eq!(caps["features"]["pendingSendTimeoutUnsend"], true);
        assert_eq!(caps["features"]["helixOwnedTemporaryId"], true);
        let commands = caps["commands"].as_array().unwrap();
        assert!(commands.iter().any(|q| q == "im_send_message"));
        assert!(!commands.iter().any(|q| q == "im_send"));
        assert!(commands.iter().any(|q| q == "im_retry_send"));
        assert!(commands.iter().any(|q| q == "im_query_dialog_list"));
        assert!(commands.iter().any(|q| q == "im_query_messages_by_channel"));
        let queries = caps["queries"].as_array().unwrap();
        assert!(queries.iter().any(|q| q == "helix_capabilities"));
        assert!(queries.iter().any(|q| q == "helix_version"));
        assert!(!queries.iter().any(|q| q == "im_query_dialog_list"));
        assert!(!queries.iter().any(|q| q == "im_query_messages_by_channel"));
        assert_eq!(caps["features"]["sendFailedProjection"], true);
        assert_eq!(caps["features"]["leftDialogListFrameV1"], true);
        let events = caps["events"].as_array().unwrap();
        assert!(events.iter().any(|e| e == "im:post:sending"));
        assert!(events.iter().any(|e| e == "im:post:received"));
        assert!(events.iter().any(|e| e == "im:post:increment-failed"));
        assert!(events.iter().any(|e| e == "im:post:send-failed"));
        assert!(events.iter().any(|e| e == "im:interaction:frame"));
    }
}
