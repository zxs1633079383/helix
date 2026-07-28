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

fn prepare_data(content_type: &str, bucket: &str) -> Value {
    json!({
        "isSuccess": true,
        "fileId": "file-1",
        "bucket": bucket,
        "method": "PUT",
        "url": "https://oss.test/media/file-1?signature=short-lived",
        "uri": "media/file-1",
        "objectKey": "media/file-1",
        "expiresAt": 1_700_000_600_000_u64,
        "headers": {
            "Content-Type": content_type,
            "x-oss-forbid-overwrite": "true"
        }
    })
}

fn envelope(data: Value) -> Tick {
    port_reply_with_body(
        0,
        enveloped(Bytes::from(
            serde_json::to_vec(&data).expect("Java media response json"),
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

fn drive_to_completion_commit(h: &mut Harness, cmd: Tick, content_type: &str, bucket: &str) -> u64 {
    h.step(cmd, 1000);
    let initial_corr = pending_stage_corr(h, "prepare");
    h.step(port_reply_ok(initial_corr), 1001);
    let prepare_corr = http_effects(h, "/oss/getPresignedUrl")[0].0;
    h.step(
        reply_with_corr(prepare_corr, envelope(prepare_data(content_type, bucket))),
        1010,
    );
    let put_corr = pending_stage_corr(h, "put");
    h.step(port_reply_ok(put_corr), 1011);
    h.step(port_reply_ok(upload_corr(h)), 1020);
    let complete_corr = pending_stage_corr(h, "complete");
    h.step(port_reply_ok(complete_corr), 1021);
    completion_commit_corr(h)
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
fn put_success_finishes_locally_without_java_complete_request() {
    let mut h = Harness::new(&[1]);
    let commit = drive_to_completion_commit(&mut h, file_cmd(), "application/pdf", "attachment");

    assert!(http_effects(&h, "/oss/completeMediaUpload").is_empty());
    assert_eq!(posts_create_count(&h), 0);
    assert_eq!(last_props(&h)["file"]["upload"]["status"], "completed");

    h.step(port_reply_ok(commit), 1031);
    assert_eq!(posts_create_count(&h), 1);
}

#[test]
fn completed_file_persists_attachment_bucket_and_uri_only() {
    let mut h = Harness::new(&[1]);
    let commit = drive_to_completion_commit(&mut h, file_cmd(), "application/pdf", "attachment");

    let props = last_props(&h);
    assert_eq!(props["file"]["bucket"], "attachment");
    assert_eq!(props["file"]["id"], "file-1");
    assert_eq!(props["file"]["name"], "doc.pdf");
    assert_eq!(props["file"]["uri"], "media/file-1");
    assert!(props["file"].get("url").is_none());
    assert_eq!(props["file"]["contentType"], "application/pdf");
    assert_eq!(props["file"]["size"], 9);
    assert_eq!(
        props["file"]["sha256"],
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
    );
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
        "signature=",
    ] {
        assert!(!persisted.contains(forbidden), "leaked {forbidden}");
    }

    h.step(port_reply_ok(commit), 1031);
    assert_eq!(posts_create_count(&h), 1);
}

#[test]
fn completed_image_keeps_render_metadata_and_uses_picture_bucket() {
    let mut h = Harness::new(&[1]);
    let commit = drive_to_completion_commit(&mut h, rich_cmd(), "image/png", "picture");

    let props = last_props(&h);
    assert_eq!(props["files"][0]["id"], "file-1");
    assert_eq!(props["files"][0]["width"], 800);
    assert_eq!(props["files"][0]["height"], 600);
    assert_eq!(props["files"][0]["bucket"], "picture");
    assert_eq!(props["files"][0]["uri"], "media/file-1");
    assert_eq!(props["files"][0]["upload"]["status"], "completed");

    h.step(port_reply_ok(commit), 1031);
    assert_eq!(posts_create_count(&h), 1);
}

#[test]
fn completion_atomic_commit_failure_never_posts_and_remains_retryable() {
    let mut h = Harness::new(&[1]);
    let commit = drive_to_completion_commit(&mut h, file_cmd(), "application/pdf", "attachment");
    h.step(common::port_reply_err(commit), 1031);

    assert_eq!(posts_create_count(&h), 0);
    assert_eq!(last_props(&h)["file"]["upload"]["status"], "failed");
    assert_eq!(
        h.module.pending_send_status("tmp-complete-file"),
        Some(helix_im::state::SendStatus::UnSend)
    );
}
