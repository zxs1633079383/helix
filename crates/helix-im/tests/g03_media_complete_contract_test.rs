mod common;

use bytes::Bytes;
use common::{cid, enveloped, port_reply_ok, port_reply_with_body, send_message_cmd_json, Harness};
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
                    "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
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
            "imagePosition": "bottom",
            "files": [{
                "width": 800,
                "height": 600,
                "mediaInput": {
                    "localPath": "/tmp/a.png",
                    "fileName": "a.png",
                    "contentType": "image/png",
                    "size": 3,
                    "sha256": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
                }
            }]
        }
    }))
}

fn prepare_data(content_type: &str) -> Value {
    json!({
        "isSuccess": true,
        "uploadToken": "upload-token-1",
        "fileId": "file-1",
        "method": "PUT",
        "url": "https://oss.test/media/file-1?signature=short-lived",
        "stablePath": "/oss/media/object/file-1",
        "objectKey": "media/file-1",
        "expiresAt": 1_700_000_600_000_u64,
        "headers": {
            "Content-Type": content_type,
            "x-oss-forbid-overwrite": "true"
        }
    })
}

fn complete_data(_file_name: &str, content_type: &str, size: u64, sha256: &str) -> Value {
    json!({
        "isSuccess": true,
        "state": "completed",
        "fileId": "file-1",
        "objectKey": "media/file-1",
        "stablePath": "/oss/media/object/file-1",
        "contentLength": size,
        "contentType": content_type,
        "sha256": sha256,
        "etag": "server-internal-etag"
    })
}

fn envelope(status: &str, data: Value) -> Tick {
    let mut response = data;
    response["isSuccess"] = json!(status == "SUCCESS");
    port_reply_with_body(
        0,
        enveloped(Bytes::from(
            serde_json::to_vec(&response).expect("Java media response json"),
        )),
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
            Effect::Persist { corr, ops } | Effect::PersistAtomic { corr, ops }
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
    let prepare_corr = http_effects(h, "/oss/getPresignedUrl")[0].0;
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
    http_effects(h, "/oss/verifyUpload")
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
            Effect::Persist { ops, .. }
            | Effect::PersistAtomic { ops, .. }
            | Effect::PersistFire { ops } => ops.as_slice(),
            _ => &[],
        })
        .filter_map(|op| match op {
            StorageOp::BatchUpsert(spec) if spec.table == "message" => Some(spec.clone()),
            _ => None,
        })
        .collect()
}

fn completion_commit_corr(h: &Harness) -> u64 {
    h.sink
        .as_slice()
        .iter()
        .find_map(|effect| {
            match effect {
            Effect::PersistAtomic { corr, ops }
                if ops.iter().any(|op| {
                    matches!(op, StorageOp::BatchDelete(spec) if spec.table == "pending_media")
                }) =>
            {
                Some(corr.raw())
            }
            _ => None,
        }
        })
        .expect("trusted media completion atomic commit")
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
fn put_success_emits_only_java_verify_with_expected_identity() {
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
    let completes = http_effects(&h, "/oss/verifyUpload");
    assert_eq!(completes.len(), 1);
    assert_eq!(completes[0].1.method, "POST");
    assert_eq!(completes[0].1.url, "http://localhost:3399/oss/verifyUpload");
    let body: Value =
        serde_json::from_slice(completes[0].1.body.as_ref().unwrap().as_ref()).unwrap();
    assert_eq!(body["mediaUpload"], true);
    assert!(body["clientUploadId"]
        .as_str()
        .is_some_and(|value| value.ends_with(":file")));
    assert_eq!(body["uploadToken"], "upload-token-1");
    assert_eq!(body["fileId"], "file-1");
    assert_eq!(body["objectKey"], "media/file-1");
    assert_eq!(body["contentType"], "application/pdf");
    assert_eq!(body["size"], 9);
    assert_eq!(
        body["sha256"],
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
    );
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
                complete_data(
                    "doc.pdf",
                    "application/pdf",
                    9,
                    "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                ),
            ),
        ),
        1030,
    );

    assert_eq!(message_upserts(&h).len(), 1);
    let props = last_props(&h);
    assert_eq!(props["file"]["bucket"], "file");
    assert_eq!(props["file"]["id"], "file-1");
    assert_eq!(props["file"]["name"], "doc.pdf");
    assert_eq!(
        props["file"]["url"],
        "http://localhost:3399/oss/media/object/file-1"
    );
    assert_eq!(props["file"]["contentType"], "application/pdf");
    assert_eq!(props["file"]["size"], 9);
    assert_eq!(
        props["file"]["sha256"],
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
    );
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
    assert_eq!(
        props["file"]["uri"],
        "http://localhost:3399/oss/media/object/file-1"
    );
    assert_eq!(posts_create_count(&h), 0);
    let commit = completion_commit_corr(&h);
    h.step(port_reply_ok(commit), 1031);
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
                complete_data(
                    "a.png",
                    "image/png",
                    3,
                    "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                ),
            ),
        ),
        1030,
    );

    let props = last_props(&h);
    assert_eq!(props["files"][0]["id"], "file-1");
    assert_eq!(props["files"][0]["width"], 800);
    assert_eq!(props["files"][0]["height"], 600);
    assert_eq!(props["files"][0]["bucket"], "file");
    assert_eq!(props["files"][0]["upload"]["status"], "completed");
    assert_eq!(posts_create_count(&h), 0);
    let commit = completion_commit_corr(&h);
    h.step(port_reply_ok(commit), 1031);
    assert_eq!(posts_create_count(&h), 1);
}

#[test]
fn complete_atomic_commit_failure_never_posts_and_remains_retryable() {
    let mut h = Harness::new(&[1]);
    let corr = drive_to_complete(&mut h, file_cmd(), "application/pdf");
    h.step(
        reply_with_corr(
            corr,
            envelope(
                "SUCCESS",
                complete_data(
                    "doc.pdf",
                    "application/pdf",
                    9,
                    "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                ),
            ),
        ),
        1030,
    );
    let commit = completion_commit_corr(&h);
    h.step(common::port_reply_err(commit), 1031);

    assert_eq!(posts_create_count(&h), 0);
    assert_eq!(last_props(&h)["file"]["upload"]["status"], "failed");
    assert_eq!(
        h.module.pending_send_status("tmp-complete-file"),
        Some(helix_im::state::SendStatus::UnSend)
    );
}

#[test]
fn complete_field_mismatch_or_missing_fails_closed() {
    let mut cases = Vec::new();
    for field in [
        "fileId",
        "objectKey",
        "stablePath",
        "contentLength",
        "sha256",
        "contentType",
        "state",
    ] {
        let mut data = complete_data(
            "doc.pdf",
            "application/pdf",
            9,
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        );
        data.as_object_mut().unwrap().remove(field);
        cases.push((format!("missing-{field}"), data));
    }
    let mismatches = [
        ("fileId", json!("other-file")),
        ("objectKey", json!("media/other-file")),
        ("stablePath", json!("/oss/media/object/other-file")),
        ("contentLength", json!(10)),
        ("sha256", json!("other-sha")),
        ("contentType", json!("image/png")),
    ];
    for (field, value) in mismatches {
        let mut data = complete_data(
            "doc.pdf",
            "application/pdf",
            9,
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        );
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
            let mut data = complete_data(
                "doc.pdf",
                "application/pdf",
                9,
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            );
            data["state"] = json!("uploaded");
            data
        }),
        envelope(
            "FAIL",
            complete_data(
                "doc.pdf",
                "application/pdf",
                9,
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            ),
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
