//! render_ready.rs 单测（issue #55·S6）：DB snake / wire camel → 同一 render-ready 形态 + 批内去重保序。

use super::shape_message_rows;
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
            "document": { "id": "doc-1" },
            "template": { "userIds": ["u9"] }
        },
        "create_at": 1700,
        "user_id": "u9"
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
    assert_eq!(r["templateReceived"], true);
    assert_eq!(r["createAt"], 1700);
    assert_eq!(r["createdAt"], 1700);
    assert_eq!(r["userId"], "u9");
    assert_eq!(r["sendStatus"], "sent");
    assert!(r["eventSeq"].is_null());
    assert_eq!(
        r["props"],
        serde_json::json!({
            "document": {"id":"doc-1"},
            "template": {"userIds":["u9"]}
        })
    );
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
        "userId": "u8"
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
    assert_eq!(r["templateReceived"], true);
    assert_eq!(r["createAt"], 1800);
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
