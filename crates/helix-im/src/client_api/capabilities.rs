use serde_json::json;

use crate::{EVENT_HTTP_UNAUTHORIZED, EVENT_NET_OFFLINE};

pub const CONTRACT_VERSION: u32 = 1;
pub const BUS_CHANNEL: &str = "im:__bus__";

pub const PUBLIC_EVENTS: &[&str] = &[
    "im:post:sending",
    "im:post:sent",
    "im:post:received",
    "im:post:updated",
    "im:post:updates",
    "im:post:revoke",
    "im:posts:batch-result",
    "im:post:read",
    "im:post:readers",
    "im:channel:read_echo",
    "im:post:deleted",
    "im:post:batch-updated",
    "im:post:send-failed",
    "im:post:increment-failed",
    "im:messages:query_result",
    "im:timeline:ready",
    "im:timeline:window",
    "im:timeline:update",
    "im:timeline:located",
    "im:timeline:thread",
    "im:channels:loaded",
    "im:channel-sync-ready",
    "im:channel-sync-page",
    "im:channel-sync-complete",
    "im:subtopics-sync-ready",
    "im:channel:update",
    "im:sync:state",
    "im:channel:created",
    "im:channel:create-failed",
    "im:channel:closed",
    "im:channel:schedule-created",
    "im:channel:schedule-canceled",
    "im:channel:member-updated",
    "im:channel:settings-updated",
    "im:channel:member-nickname",
    "im:channel:members",
    "im:channel:increment",
    "im:channel:replies",
    "im:announcement:list-updated",
    "im:operation:status",
    "im:user:candidates",
    "im:read:result",
    "im:todo:updated",
    "im:post_chain:draft",
    "im:post_chain:publish",
    "im:post_chain:upsert",
    "im:post_chain:append_rejected",
    "im:post_chain:reconcile",
    "im:post_chain:close",
    "im:post_chain:retract",
    "im:post_chain:read_cursor",
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
            "messageV3Events": true,
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
        assert_eq!(caps["features"]["messageV3Events"], true);
        let events = caps["events"].as_array().unwrap();
        assert!(events.iter().any(|e| e == "im:post:sending"));
        assert!(events.iter().any(|e| e == "im:post:received"));
        assert!(events.iter().any(|e| e == "im:post:increment-failed"));
        assert!(events.iter().any(|e| e == "im:post:send-failed"));
        assert!(events.iter().any(|e| e == "im:timeline:window"));
        assert!(events.iter().any(|e| e == "im:timeline:ready"));
        assert!(events.iter().any(|e| e == "im:operation:status"));
        assert!(events.iter().any(|e| e == "im:channel:member-updated"));
        assert!(!events.iter().any(|e| e == "im:channel:left"));
        assert!(events.iter().any(|e| e == "im:channel:increment"));
        assert!(events.iter().any(|e| e == "im:announcement:list-updated"));
    }

    /// 证明 Phase 1 Post 词表包含单条撤回和批量绝对态 Upsert。
    #[test]
    fn capabilities_advertise_phase1_post_projection_events() {
        let caps: serde_json::Value = serde_json::from_slice(&capabilities_json()).unwrap();
        let events = caps["events"].as_array().unwrap();
        for expected in [
            "im:post:sending",
            "im:post:received",
            "im:post:updated",
            "im:post:updates",
            "im:post:send-failed",
            "im:post:read",
            "im:post:readers",
            "im:post:revoke",
        ] {
            assert!(
                events.iter().any(|event| event == expected),
                "missing Phase 1 event {expected}"
            );
        }
    }
}
