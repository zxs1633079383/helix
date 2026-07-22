//! render_ready.rs 单测（issue #55·S6）：DB snake / wire camel → 同一 render-ready 形态 + 批内去重保序。

use super::{shape_message_rows, shape_message_rows_for_viewer};
use serde_json::json;

#[test]
fn db_snake_row_shaped_to_render_ready() {
    // 首屏 Scan(message) 的 DB snake 列。
    let rows = json!([{
        "temporary_id": "lf-t1",
        "id": "srv1",
        "channel_id": "ch1",
        "type": "DOCUMENT",
        "message": "hi",
        "read_bits": 5,
        "revoke": 1,
        "props": {
            "channel_event_seq": 42,
            "document": { "id": "doc-1" },
            "template": { "userIds": ["u9"] }
        },
        "create_at": 1700,
        "user_id": "u9",
        "reply_id": "root-1",
        "reply_root_id": "root-1",
        "reply_first_level_id": "first-1"
    }]);
    let out = shape_message_rows(&rows);
    let r = &out[0];
    assert_eq!(r["msgId"], "srv1"); // server id 优先
    assert_eq!(r["id"], "srv1");
    assert_eq!(r["temporaryId"], "lf-t1");
    assert_eq!(r["channelId"], "ch1");
    assert_eq!(r["text"], "hi");
    assert_eq!(r["type"], "DOCUMENT");
    assert_eq!(r["readBits"], "5"); // number→string 归一
    assert_eq!(r["revoked"], true); // revoke 1→true
    assert_eq!(r["revoke"], true);
    assert_eq!(r["actionCapabilities"]["canLocate"], false);
    assert_eq!(r["templateReceived"], true);
    assert_eq!(r["createAt"], 1700);
    assert_eq!(r["createdAt"], 1700);
    assert_eq!(r["userId"], "u9");
    assert_eq!(r["sendStatus"], "sent");
    assert_eq!(r["replyId"], "root-1");
    assert_eq!(r["replyRootId"], "root-1");
    assert_eq!(r["replyFirstLevelId"], "first-1");
    assert_eq!(r["eventSeq"], 42);
    assert_eq!(
        r["props"],
        serde_json::json!({
            "channel_event_seq": 42,
            "document": {"id":"doc-1"},
            "template": {"userIds":["u9"]}
        })
    );
}

#[test]
fn db_text_props_restore_channel_event_seq_for_timeline_vm() {
    let rows = json!([{
        "id": "server-event-seq",
        "channel_id": "ch1",
        "message": "persisted",
        "props": "{\"channel_event_seq\":7}"
    }]);

    let out = shape_message_rows(&rows);
    assert_eq!(out[0]["eventSeq"], 7);
}

#[test]
fn historical_wire_creator_survives_render_ready_shaping() {
    let rows = json!([{
        "id": "legacy-reconnect",
        "channelId": "ch1",
        "message": "recovered",
        "userSnapshot": { "userId": "" },
        "createBy": "SYS"
    }]);

    let out = shape_message_rows(&rows);
    assert_eq!(out[0]["userId"], "SYS");
}

#[test]
fn durable_unsend_is_rendered_as_retryable_failed_after_restart() {
    let rows = json!([{
        "temporary_id": "tmp-failed",
        "id": "",
        "channel_id": "ch1",
        "type": "TEXT",
        "message": "retry me",
        "send_status": "unsend"
    }]);
    let out = shape_message_rows(&rows);
    assert_eq!(out[0]["id"], "tmp-failed");
    assert_eq!(out[0]["sendStatus"], "failed");
    assert_eq!(out[0]["actionCapabilities"]["canRetry"], true);
}

#[test]
fn persisted_reply_preview_restores_missing_reply_count() {
    let rows = json!([{
        "id": "root-with-replies",
        "channel_id": "ch1",
        "message": "原消息",
        "reply_count": 0,
        "reply_messages": {
            "reply-1": {"message": "一"},
            "reply-2": {"message": "二"}
        }
    }]);

    let out = shape_message_rows(&rows);
    assert_eq!(out[0]["replyCount"], 2);
    assert_eq!(
        out[0]["replyMessages"].as_object().map(|items| items.len()),
        Some(2)
    );
}

#[test]
fn persisted_authoritative_reply_count_is_not_reduced_to_preview_size() {
    let rows = json!([{
        "id": "root-with-many-replies",
        "reply_count": 9,
        "reply_messages": {"reply-1": {"message": "预览"}}
    }]);

    let out = shape_message_rows(&rows);
    assert_eq!(out[0]["replyCount"], 9);
}

#[test]
fn viewer_relative_actions_and_absolute_interaction_counts_are_render_ready() {
    let rows = json!([{
        "temporary_id": "tmp-1",
        "id": "server-1",
        "user_id": "444",
        "props": {
            "quickReply": [{"emoji":"👍","userIds":["444","678"]}],
            "template": {"userIds":["444","678"]}
        },
        "expedite_map": "{\"sender\":{\"id\":\"444\"},\"recipients\":{\"678\":{\"status\":0},\"789\":{\"status\":1}}}"
    }]);
    let out = shape_message_rows_for_viewer(&rows, "444");
    assert_eq!(out[0]["isSelf"], true);
    assert_eq!(out[0]["actionCapabilities"]["canRevoke"], true);
    assert_eq!(out[0]["reactions"], "👍");
    assert_eq!(out[0]["reactionCount"], 2);
    assert_eq!(out[0]["templateConfirmedCount"], 2);
    assert_eq!(out[0]["readerIds"], json!(["444", "678"]));
    assert_eq!(out[0]["urgentTargetCount"], 2);
    assert_eq!(out[0]["urgentConfirmedCount"], 1);
    assert_eq!(out[0]["urgentRequesterId"], "444");
    assert_eq!(out[0]["urgentTargetIds"], json!(["678", "789"]));
    assert_eq!(out[0]["urgentConfirmedIds"], json!(["789"]));
    assert_eq!(out[0]["urgentRequiredCount"], 2);
    assert_eq!(out[0]["urgentState"], "active");
    assert_eq!(out[0]["canConfirmUrgent"], false);
}

#[test]
fn storage_map_quick_reply_is_shaped_like_ws_array() {
    let rows = json!([{
        "id": "server-map-reaction",
        "props": {"quickReply": {":thumbsup:": ["444", "678"]}}
    }]);

    let out = shape_message_rows_for_viewer(&rows, "444");
    assert_eq!(out[0]["reactions"], ":thumbsup:");
    assert_eq!(out[0]["reactionCount"], 2);
}

#[test]
fn wire_camel_row_shaped_to_same_shape() {
    // 上拉 postContext 的 wire camel Post（无 revoke 列·空 type 容错 TEXT）。
    let rows = json!([{
        "temporaryId": "lf-t2",
        "id": "srv2",
        "channelId": "ch2",
        "type": "",
        "message": "yo",
        "readBits": "3",
        "templateReceived": true,
        "createAt": 1800,
        "userId": "u8",
        "replyId": "root-2",
        "replyRootId": "root-2",
        "replyFirstLevelId": "first-2"
    }]);
    let out = shape_message_rows(&rows);
    let r = &out[0];
    assert_eq!(r["msgId"], "srv2");
    assert_eq!(r["temporaryId"], "lf-t2");
    assert_eq!(r["channelId"], "ch2");
    assert_eq!(r["text"], "yo");
    assert_eq!(r["type"], "TEXT"); // 空 type 容错
    assert_eq!(r["readBits"], "3");
    assert_eq!(r["revoked"], false); // 无 revoke 列→false
    assert_eq!(r["actionCapabilities"]["canLocate"], true);
    assert_eq!(r["templateReceived"], true);
    assert_eq!(r["createAt"], 1800);
    assert_eq!(r["replyId"], "root-2");
    assert_eq!(r["replyRootId"], "root-2");
    assert_eq!(r["replyFirstLevelId"], "first-2");
}

#[test]
fn template_received_derived_from_props_json_string() {
    let rows = json!([{
        "temporary_id": "lf-template",
        "message": "template",
        "props": "{\"template\":{\"user_ids\":[\"u1\"]}}"
    }]);
    let out = shape_message_rows(&rows);
    assert_eq!(out[0]["templateReceived"], true);
}

#[test]
fn server_id_missing_falls_back_to_temporary_id() {
    let rows = json!([{ "temporary_id": "lf-only", "message": "m" }]);
    let out = shape_message_rows(&rows);
    assert_eq!(out[0]["msgId"], "lf-only");
}

#[test]
fn dedup_by_msg_id_keeps_order() {
    // 同 server id 重复（多轮上拉重叠 / 与首屏重叠）→ 批内去重·保序（首现保留）。
    let rows = json!([
        { "id": "a", "message": "1", "createAt": 100 },
        { "id": "b", "message": "2", "createAt": 200 },
        { "id": "a", "message": "1-dup", "createAt": 100 },
    ]);
    let out = shape_message_rows(&rows);
    assert_eq!(out.as_array().unwrap().len(), 2);
    assert_eq!(out[0]["msgId"], "a");
    assert_eq!(out[0]["text"], "1"); // 首现保留（非 dup）
    assert_eq!(out[1]["msgId"], "b");
}

#[test]
fn non_object_and_anchorless_rows_skipped() {
    let rows = json!([42, "str", {}, { "message": "no-id" }]);
    let out = shape_message_rows(&rows);
    // {} 与无锚行（无 id/temporary_id → msgId 空）跳过 → 0 行。
    assert_eq!(out.as_array().unwrap().len(), 0);
}

#[test]
fn non_array_input_yields_empty() {
    assert_eq!(shape_message_rows(&json!({})), json!([]));
    assert_eq!(shape_message_rows(&json!(null)), json!([]));
}

#[test]
fn merged_forward_history_row_contains_same_render_ready_detail() {
    let items = json!([{
        "temporaryId":"forward-1",
        "message":"历史转发正文",
        "type":"TEXT",
        "createAt":9,
        "userSnapshot":{"userName":"乙"}
    }]);
    let rows = json!([{
        "id":"merged-1",
        "temporary_id":"merged-tmp-1",
        "type":"MULTIPLY",
        "props":{"mergeMessage":items}
    }]);

    let out = shape_message_rows(&rows);
    let detail = &out[0]["forwardDetail"];
    assert_eq!(detail["mode"], "merged");
    assert_eq!(detail["state"], "ready");
    assert_eq!(detail["items"][0]["id"], "forward-1");
    assert_eq!(detail["items"][0]["author"], "乙");
    assert_eq!(detail["items"][0]["text"], "历史转发正文");
}
