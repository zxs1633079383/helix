mod common;

use bytes::Bytes;
use common::{cid, port_reply_ok, port_reply_with_body, send_message_cmd_json, Harness};
use helix_core::effect::{Effect, HttpRequest, SqlValue, StorageOp, UpsertSpec};
use helix_core::Tick;
use serde_json::{json, Value};

fn file_cmd() -> Tick {
    send_message_cmd_json(json!({
        "channel_id": cid(1),
        "temporary_id": "tmp-complete-file",
        "type": "file",
        "text": "",
        "props": {
            "file": {
                "mediaInput": {
                    "localPath": "/tmp/doc.pdf",
                    "fileName": "doc.pdf",
                    "contentType": "application/pdf",
                    "size": 9,
                    "sha256": "sha256-fixture"
                }
            },
            "frontendCustom": {"keep": true}
        }
    }))
}

fn rich_cmd() -> Tick {
    send_message_cmd_json(json!({
        "channel_id": cid(1),
        "temporary_id": "tmp-complete-rich",
        "type": "rich",
        "text": "hello",
        "props": {
            "textPosition": "bottom",
            "images": [{
                "width": 800,
                "height": 600,
                "mediaInput": {
                    "localPath": "/tmp/a.png",
                    "fileName": "a.png",
                    "contentType": "image/png",
                    "size": 3,
                    "sha256": "rich-sha256"
                }
            }]
        }
    }))
}

fn prepare_data(content_type: &str) -> Value {
    json!({
        "uploadId": "upload-1",
        "fileId": "file-1",
        "method": "PUT",
        "url": "http://test/media/upload/upload-1",
        "publicUrl": "http://test/media/object/file-1",
        "objectKey": "object-internal-file-1",
        "headers": {"Content-Type": content_type}
    })
}

fn complete_data(file_name: &str, content_type: &str, size: u64, sha256: &str) -> Value {
    json!({
        "id": "file-1",
        "uploadId": "upload-1",
        "fileId": "file-1",
        "url": "http://test/media/object/file-1",
        "size": size,
        "contentType": content_type,
        "fileName": file_name,
        "sha256": sha256,
        "channelId": cid(1),
        "userId": "server-owned-user",
        "teamId": "server-owned-team",
        "etag": "server-internal-etag",
        "state": "completed"
    })
}

fn envelope(status: &str, data: Value) -> Tick {
    port_reply_with_body(
        0,
        Bytes::from(
            serde_json::to_vec(&json!({"status": status, "data": data})).expect("CommonRes json"),
        ),
    )
}

fn http_effects(h: &Harness, suffix: &str) -> Vec<(u64, HttpRequest)> {
    h.sink
        .as_slice()
        .iter()
        .filter_map(|effect| match effect {
            Effect::Http { corr, req } if req.url.ends_with(suffix) => {
                Some((corr.raw(), req.clone()))
            }
            _ => None,
        })
        .collect()
}

fn upload_corr(h: &Harness) -> u64 {
    h.sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::UploadFile { corr, .. } => Some(corr.raw()),
            _ => None,
        })
        .expect("UploadFile corr")
}

fn pending_stage_corr(h: &Harness, stage: &str) -> u64 {
    h.sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Persist { corr, ops }
                if ops.iter().any(|op| match op {
                    StorageOp::BatchUpsert(spec) if spec.table == "pending_media" => {
                        spec.rows.iter().flatten().any(|(column, value)| {
                            column == "stage"
                                && matches!(value, SqlValue::Text(value) if value == stage)
                        })
                    }
                    _ => false,
                }) =>
            {
                Some(corr.raw())
            }
            _ => None,
        })
        .expect("pending media stage persist")
}

fn reply_with_corr(corr: u64, tick: Tick) -> Tick {
    match tick {
        Tick::PortReply { outcome, .. } => Tick::PortReply {
            corr: helix_core::Correlation::from_raw(corr),
            outcome,
        },
        _ => unreachable!(),
    }
}

fn drive_to_put(h: &mut Harness, cmd: Tick, content_type: &str) {
    h.step(cmd, 1000);
    let initial_corr = pending_stage_corr(h, "prepare");
    h.step(port_reply_ok(initial_corr), 1001);
    let prepare_corr = http_effects(h, "/media/upload/prepare")[0].0;
    h.step(
        reply_with_corr(
            prepare_corr,
            envelope("SUCCESS", prepare_data(content_type)),
        ),
        1010,
    );
    let put_corr = pending_stage_corr(h, "put");
    h.step(port_reply_ok(put_corr), 1011);
}

fn drive_to_complete(h: &mut Harness, cmd: Tick, content_type: &str) -> u64 {
    drive_to_put(h, cmd, content_type);
    h.step(port_reply_ok(upload_corr(h)), 1020);
    let complete_corr = pending_stage_corr(h, "complete");
    h.step(port_reply_ok(complete_corr), 1021);
    http_effects(h, "/media/upload/complete")
        .last()
        .expect("complete request")
        .0
}

fn posts_create_count(h: &Harness) -> usize {
    http_effects(h, "/posts/create").len()
}

fn message_upserts(h: &Harness) -> Vec<UpsertSpec> {
    h.sink
        .as_slice()
        .iter()
        .flat_map(|effect| match effect {
            Effect::Persist { ops, .. } | Effect::PersistFire { ops } => ops.as_slice(),
            _ => &[],
        })
        .filter_map(|op| match op {
            StorageOp::BatchUpsert(spec) if spec.table == "message" => Some(spec.clone()),
            _ => None,
        })
        .collect()
}

fn row_text<'a>(row: &'a [(String, SqlValue)], key: &str) -> Option<&'a str> {
    row.iter().find_map(|(column, value)| match value {
        SqlValue::Text(text) if column == key => Some(text.as_str()),
        _ => None,
    })
}

fn last_props(h: &Harness) -> Value {
    let spec = message_upserts(h).pop().expect("message upsert");
    serde_json::from_str(row_text(&spec.rows[0], "props").expect("props")).expect("props json")
}

#[test]
fn put_success_emits_only_complete_with_upload_id() {
    let mut h = Harness::new(&[1]);
    drive_to_put(&mut h, file_cmd(), "application/pdf");

    h.step(port_reply_ok(upload_corr(&h)), 1020);
    let complete_corr = pending_stage_corr(&h, "complete");
    let effects = h.step(port_reply_ok(complete_corr), 1021);

    assert_eq!(
        effects
            .iter()
            .filter(|effect| matches!(effect, common::EffectKind::Http(_)))
            .count(),
        1
    );
    let completes = http_effects(&h, "/media/upload/complete");
    assert_eq!(completes.len(), 1);
    assert_eq!(completes[0].1.method, "POST");
    assert_eq!(completes[0].1.url, "http://test/media/upload/complete");
    let body: Value =
        serde_json::from_slice(completes[0].1.body.as_ref().unwrap().as_ref()).unwrap();
    assert_eq!(body, json!({"uploadId": "upload-1"}));
    assert_eq!(posts_create_count(&h), 0);
}

#[test]
fn complete_success_persists_trusted_reference_before_posts_create() {
    let mut h = Harness::new(&[1]);
    let corr = drive_to_complete(&mut h, file_cmd(), "application/pdf");

    h.step(
        reply_with_corr(
            corr,
            envelope(
                "SUCCESS",
                complete_data("doc.pdf", "application/pdf", 9, "sha256-fixture"),
            ),
        ),
        1030,
    );

    assert_eq!(message_upserts(&h).len(), 1);
    let props = last_props(&h);
    assert_eq!(props["file"]["id"], "file-1");
    assert_eq!(props["file"]["name"], "doc.pdf");
    assert_eq!(props["file"]["url"], "http://test/media/object/file-1");
    assert_eq!(props["file"]["contentType"], "application/pdf");
    assert_eq!(props["file"]["size"], 9);
    assert_eq!(props["file"]["sha256"], "sha256-fixture");
    assert_eq!(props["file"]["upload"]["status"], "completed");
    assert_eq!(props["frontendCustom"]["keep"], true);
    let persisted = serde_json::to_string(&props).unwrap();
    for forbidden in [
        "mediaInput",
        "localPath",
        "objectKey",
        "uploadId",
        "fileId",
        "headers",
        "userId",
        "teamId",
        "etag",
        "/tmp/doc.pdf",
    ] {
        assert!(!persisted.contains(forbidden), "leaked {forbidden}");
    }
    assert_eq!(posts_create_count(&h), 1);
}

#[test]
fn rich_complete_preserves_render_metadata_in_posts_create() {
    let mut h = Harness::new(&[1]);
    let corr = drive_to_complete(&mut h, rich_cmd(), "image/png");
    h.step(
        reply_with_corr(
            corr,
            envelope(
                "SUCCESS",
                complete_data("a.png", "image/png", 3, "rich-sha256"),
            ),
        ),
        1030,
    );

    let props = last_props(&h);
    assert_eq!(props["images"][0]["id"], "file-1");
    assert_eq!(props["images"][0]["width"], 800);
    assert_eq!(props["images"][0]["height"], 600);
    assert_eq!(props["images"][0]["upload"]["status"], "completed");
    assert_eq!(posts_create_count(&h), 1);
}

#[test]
fn complete_field_mismatch_or_missing_fails_closed() {
    let mut cases = Vec::new();
    for field in [
        "uploadId",
        "fileId",
        "size",
        "sha256",
        "contentType",
        "channelId",
        "url",
        "fileName",
        "state",
    ] {
        let mut data = complete_data("doc.pdf", "application/pdf", 9, "sha256-fixture");
        data.as_object_mut().unwrap().remove(field);
        cases.push((format!("missing-{field}"), data));
    }
    let mismatches = [
        ("uploadId", json!("other-upload")),
        ("fileId", json!("other-file")),
        ("size", json!(10)),
        ("sha256", json!("other-sha")),
        ("contentType", json!("image/png")),
        ("channelId", json!(cid(2))),
        ("url", json!("http://test/media/object/other-file")),
        ("fileName", json!("other.pdf")),
    ];
    for (field, value) in mismatches {
        let mut data = complete_data("doc.pdf", "application/pdf", 9, "sha256-fixture");
        data[field] = value;
        cases.push((format!("mismatch-{field}"), data));
    }

    for (name, data) in cases {
        let mut h = Harness::new(&[1]);
        let corr = drive_to_complete(&mut h, file_cmd(), "application/pdf");
        h.step(reply_with_corr(corr, envelope("SUCCESS", data)), 1030);
        assert_eq!(
            message_upserts(&h).len(),
            1,
            "{name} should persist failed status"
        );
        assert_eq!(
            last_props(&h)["file"]["upload"]["status"],
            "failed",
            "{name}"
        );
        assert_eq!(posts_create_count(&h), 0, "{name}");
    }
}

#[test]
fn complete_failed_state_status_or_malformed_body_fails_closed() {
    let replies = [
        envelope("SUCCESS", {
            let mut data = complete_data("doc.pdf", "application/pdf", 9, "sha256-fixture");
            data["state"] = json!("uploaded");
            data
        }),
        envelope(
            "FAIL",
            complete_data("doc.pdf", "application/pdf", 9, "sha256-fixture"),
        ),
        port_reply_with_body(0, Bytes::from_static(b"{bad json")),
    ];

    for reply in replies {
        let mut h = Harness::new(&[1]);
        let corr = drive_to_complete(&mut h, file_cmd(), "application/pdf");
        h.step(reply_with_corr(corr, reply), 1030);
        assert_eq!(last_props(&h)["file"]["upload"]["status"], "failed");
        assert_eq!(posts_create_count(&h), 0);
    }
}
