use super::*;
use serde_json::{json, Map, Value};

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

    let branch = normalize_command_payload("im_get_reply_branch", br#"{"postId":"p2"}"#).unwrap();
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
fn capabilities_advertise_the_send_failed_projection_emitted_by_core() {
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
    assert_eq!(caps["features"]["sendFailedProjection"], true);
    let events = caps["events"].as_array().unwrap();
    assert!(events.iter().any(|e| e == "im:post:sending"));
    assert!(events.iter().any(|e| e == "im:post:received"));
    assert!(events.iter().any(|e| e == "im:post:send-failed"));
}

#[test]
fn outbound_command_names_is_publicly_reexported() {
    assert_eq!(
        crate::outbound::outbound_command_names(),
        crate::outbound::registry::outbound_command_names()
    );
}
