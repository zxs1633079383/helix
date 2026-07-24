//! G-03 durable media recovery contract.
//!
//! These tests model a process restart as a fresh `ImModule` receiving the
//! persisted `pending_media` scan before the user invokes `im_retry_send`.

mod common;

use bytes::Bytes;
use common::{
    cid, command_json, enveloped, port_reply_err, port_reply_ok, port_reply_with_body,
    send_message_cmd_json, Harness,
};
use helix_core::effect::{Effect, HttpRequest, SqlValue, StorageOp};
use helix_core::Tick;
use serde_json::{json, Value};

const TMP: &str = "tmp-durable-file";

fn file_command() -> Tick {
    send_message_cmd_json(json!({
        "channel_id": cid(1),
        "temporary_id": TMP,
        "type": "file",
        "text": "",
        "props": {
            "file": {"mediaInput": media_input("doc.pdf", "application/pdf", 9, "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")},
            "frontendCustom": {"keep": true}
        }
    }))
}

fn media_input(name: &str, content_type: &str, size: u64, sha256: &str) -> Value {
    json!({
        "localPath": format!("/tmp/{name}"),
        "fileName": name,
        "contentType": content_type,
        "size": size,
        "sha256": sha256
    })
}

fn pending_row(
    temporary_id: &str,
    target_key: &str,
    stage: &str,
    name: &str,
    content_type: &str,
    size: u64,
    sha256: &str,
    upload_id: &str,
    file_id: &str,
) -> Value {
    json!({
        "temporary_id": temporary_id,
        "target_key": target_key,
        "stage": stage,
        "channel_id": cid(1),
        "local_path": format!("/tmp/{name}"),
        "file_name": name,
        "content_type": content_type,
        "size": size,
        "sha256": sha256,
        "upload_id": upload_id,
        "file_id": file_id,
        "method": if upload_id.is_empty() { "" } else { "PUT" },
        "upload_url": "",
        "public_url": if file_id.is_empty() { "".to_string() } else { format!("http://localhost:3399/oss/media/object/{file_id}") },
        "object_key": if file_id.is_empty() { "".to_string() } else { format!("media/{file_id}") },
        "headers_json": "[]"
    })
}

fn message_row(temporary_id: &str, msg_type: &str, props: Value, status: &str) -> Value {
    json!([{
        "temporary_id": temporary_id,
        "id": "",
        "channel_id": cid(1),
        "user_id": "user-1",
        "type": msg_type,
        "message": if matches!(msg_type, "rich" | "IMAGE") { "hello" } else { "" },
        "props": serde_json::to_string(&props).unwrap(),
        "viewers": "[]",
        "mentions": "[]",
        "user_snapshot": "{}",
        "topic": "",
        "reply_id": "",
        "reply_root_id": "",
        "reply_first_level_id": "",
        "simple_message": if matches!(msg_type, "rich" | "IMAGE") {
            "[图片]hello"
        } else {
            "[文件]doc.pdf"
        },
        "create_at": 1000,
        "send_status": status
    }])
}

fn pending_scan_corr(h: &Harness) -> u64 {
    h.sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Persist { corr, ops }
                if ops.iter().any(
                    |op| matches!(op, StorageOp::Scan(spec) if spec.table == "pending_media"),
                ) =>
            {
                Some(corr.raw())
            }
            _ => None,
        })
        .expect("on_start must scan pending_media")
}

fn persist_corr_for_get(h: &Harness, table: &str) -> u64 {
    h.sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Persist { corr, ops }
                if ops
                    .iter()
                    .any(|op| matches!(op, StorageOp::Get(spec) if spec.table == table)) =>
            {
                Some(corr.raw())
            }
            _ => None,
        })
        .expect("expected durable get")
}

fn persist_corr_for_upsert_stage(h: &Harness, stage: &str) -> u64 {
    h.sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Persist { corr, ops } | Effect::PersistAtomic { corr, ops }
                if ops.iter().any(|op| match op {
                    StorageOp::BatchUpsert(spec) if spec.table == "pending_media" => {
                        spec.rows.iter().any(|row| {
                            row.iter().any(|(column, value)| {
                                column == "stage"
                                    && matches!(value, SqlValue::Text(value) if value == stage)
                            })
                        })
                    }
                    _ => false,
                }) =>
            {
                Some(corr.raw())
            }
            _ => None,
        })
        .expect("expected pending_media stage persist")
}

fn retry_reset_corr(h: &Harness) -> Option<u64> {
    h.sink.as_slice().iter().find_map(|effect| match effect {
        Effect::PersistAtomic { corr, ops }
            if ops.iter().any(|op| match op {
                StorageOp::BatchUpsert(spec) if spec.table == "message" => {
                    spec.rows.iter().flatten().any(|(column, value)| {
                        column == "send_status"
                            && matches!(value, SqlValue::Text(value) if value == "sending")
                    })
                }
                _ => false,
            }) =>
        {
            Some(corr.raw())
        }
        _ => None,
    })
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

fn recovery_compensation_corr(h: &Harness) -> Option<u64> {
    h.sink.as_slice().iter().find_map(|effect| match effect {
        Effect::PersistAtomic { corr, ops }
            if ops.iter().any(|op| match op {
                StorageOp::BatchUpdate(spec) if spec.table == "message" => {
                    spec.patch.iter().any(|(column, value)| {
                        column == "send_status"
                            && matches!(value, SqlValue::Text(value) if value == "unsend")
                    })
                }
                _ => false,
            }) =>
        {
            Some(corr.raw())
        }
        _ => None,
    })
}

fn restart_with(rows: Vec<Value>) -> Harness {
    let mut h = Harness::new(&[1]);
    h.start(0);
    let corr = pending_scan_corr(&h);
    h.step(
        port_reply_with_body(corr, Bytes::from(serde_json::to_vec(&rows).unwrap())),
        1,
    );
    if let Some(compensation) = recovery_compensation_corr(&h) {
        h.step(port_reply_ok(compensation), 2);
    }
    h
}

fn retry_with_message(h: &mut Harness, temporary_id: &str, row: Value) {
    h.step(
        command_json("im_retry_send", json!({"message_id": temporary_id})),
        10,
    );
    let corr = persist_corr_for_get(h, "message");
    h.step(
        port_reply_with_body(corr, Bytes::from(serde_json::to_vec(&row).unwrap())),
        11,
    );
    if let Some(reset) = retry_reset_corr(h) {
        h.step(port_reply_ok(reset), 12);
    }
}

fn http(h: &Harness, suffix: &str) -> Vec<(u64, HttpRequest)> {
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

fn upload_count(h: &Harness) -> usize {
    h.sink
        .as_slice()
        .iter()
        .filter(|effect| matches!(effect, Effect::UploadFile { .. }))
        .count()
}

fn complete_success(_upload_id: &str, file_id: &str, name: &str, size: u64, sha: &str) -> Tick {
    let response = Bytes::from(
        serde_json::to_vec(&json!({
            "isSuccess": true,
            "state": "completed",
            "fileId": file_id,
            "objectKey": format!("media/{file_id}"),
            "stablePath": format!("/oss/media/object/{file_id}"),
            "contentLength": size,
            "contentType": if name.ends_with(".png") { "image/png" } else { "application/pdf" },
            "sha256": sha,
            "etag": "etag-internal"
        }))
        .unwrap(),
    );
    port_reply_with_body(0, enveloped(response))
}

fn prepare_success(data: Value) -> Bytes {
    enveloped(Bytes::from(serde_json::to_vec(&data).unwrap()))
}

fn with_corr(corr: u64, tick: Tick) -> Tick {
    match tick {
        Tick::PortReply { outcome, .. } => Tick::PortReply {
            corr: helix_core::Correlation::from_raw(corr),
            outcome,
        },
        _ => unreachable!(),
    }
}

#[test]
fn prepare_stage_is_persisted_in_internal_table_without_public_props_leak() {
    let mut h = Harness::new(&[1]);
    h.step(file_command(), 1000);

    let mut pending = None;
    let mut public_props = None;
    for effect in h.sink.as_slice() {
        let ops = match effect {
            Effect::Persist { ops, .. } | Effect::PersistAtomic { ops, .. } => ops,
            _ => continue,
        };
        for op in ops {
            if let StorageOp::BatchUpsert(spec) = op {
                if spec.table == "pending_media" {
                    pending = Some(spec.clone());
                }
                if spec.table == "message" {
                    public_props = spec.rows[0].iter().find_map(|(column, value)| match value {
                        SqlValue::Text(text) if column == "props" => Some(text.clone()),
                        _ => None,
                    });
                }
            }
        }
    }
    let pending = pending.expect("prepare stage must be durable");
    assert!(pending.rows[0].iter().any(|(column, value)| {
        column == "stage" && matches!(value, SqlValue::Text(stage) if stage == "prepare")
    }));
    let public_props = public_props.expect("public message props");
    assert!(!public_props.contains("mediaInput"));
    assert!(!public_props.contains("/tmp/doc.pdf"));
}

#[test]
fn startup_recovery_atomically_marks_orphan_messages_retryable_before_opening_retry() {
    let mut h = Harness::new(&[1]);
    h.start(0);
    let scan = pending_scan_corr(&h);
    h.step(
        port_reply_with_body(
            scan,
            Bytes::from(
                serde_json::to_vec(&vec![pending_row(
                    TMP,
                    "file",
                    "put",
                    "doc.pdf",
                    "application/pdf",
                    9,
                    "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                    "upload-1",
                    "file-1",
                )])
                .unwrap(),
            ),
        ),
        1,
    );

    let compensation = recovery_compensation_corr(&h)
        .expect("orphan journal must first produce an atomic message compensation");
    assert!(h.sink.as_slice().iter().any(|effect| match effect {
        Effect::PersistAtomic { corr, ops } if corr.raw() == compensation => {
            ops.iter().any(|op| match op {
                StorageOp::BatchUpdate(spec) if spec.table == "message" => {
                    spec.key_vals.len() == 1
                        && matches!(
                            &spec.key_vals[0],
                            SqlValue::Text(value) if value == TMP
                        )
                        && spec.patch.iter().any(|(column, value)| {
                            column == "upload_progress_percent"
                                && matches!(value, SqlValue::Integer(0))
                        })
                }
                _ => false,
            })
        }
        _ => false,
    }));
    assert!(http(&h, "/oss/getPresignedUrl").is_empty());
    assert!(http(&h, "/oss/completeMediaUpload").is_empty());
}

#[test]
fn initial_pending_media_persist_failure_releases_spinner_without_starting_prepare() {
    let mut h = Harness::new(&[1]);
    h.step(file_command(), 1000);
    let initial_corr = persist_corr_for_upsert_stage(&h, "prepare");
    assert!(http(&h, "/oss/getPresignedUrl").is_empty());

    h.step(port_reply_err(initial_corr), 1001);
    assert!(http(&h, "/oss/getPresignedUrl").is_empty());
    assert_eq!(upload_count(&h), 0);
    assert!(h.sink.as_slice().iter().any(|effect| {
        let Effect::Emit { event } = effect else {
            return false;
        };
        serde_json::from_slice::<Value>(event.0.as_ref()).is_ok_and(|value| {
            value["event"] == "im:post:send-failed" && value["data"]["temporaryId"] == TMP
        })
    }));
    let compensation = h
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::PersistAtomic { corr, ops }
                if corr.raw() != initial_corr
                    && ops.iter().any(|op| {
                        matches!(
                            op,
                            StorageOp::BatchUpsert(spec)
                                if spec.table == "message"
                                    && spec.rows.iter().flatten().any(|(column, value)|
                                        column == "send_status"
                                            && matches!(value, SqlValue::Text(status) if status == "unsend")
                                    )
                        )
                    }) =>
            {
                Some((corr.raw(), ops))
            }
            _ => None,
        })
        .expect("media failure compensation must be atomic");
    assert!(compensation.1.iter().any(|op| {
        matches!(
            op,
            StorageOp::BatchUpsert(spec)
                if spec.table == "pending_media"
                    && spec.rows.iter().flatten().any(|(column, value)|
                        column == "stage"
                            && matches!(value, SqlValue::Text(stage) if stage == "prepare")
                    )
        )
    }));
    assert_eq!(
        h.module.pending_send_status(TMP),
        Some(helix_im::state::SendStatus::UnSend)
    );

    h.step(port_reply_ok(compensation.0), 1002);
    let mut restarted = restart_with(vec![pending_row(
        TMP,
        "file",
        "prepare",
        "doc.pdf",
        "application/pdf",
        9,
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "",
        "",
    )]);
    retry_with_message(
        &mut restarted,
        TMP,
        message_row(
            TMP,
            "file",
            json!({"file":{"upload":{"status":"failed"}}}),
            "unsend",
        ),
    );
    assert_eq!(http(&restarted, "/oss/getPresignedUrl").len(), 1);
    assert!(http(&restarted, "/posts/create").is_empty());
}

#[test]
fn prepare_and_put_stage_persist_errors_never_advance_external_io() {
    let mut h = Harness::new(&[1]);
    h.step(file_command(), 1000);
    let initial = persist_corr_for_upsert_stage(&h, "prepare");
    h.step(port_reply_ok(initial), 1001);
    let prepare = http(&h, "/oss/getPresignedUrl")[0].0;
    let prepare_reply = port_reply_with_body(
        prepare,
        prepare_success(json!({
            "isSuccess":true, "uploadToken":"upload-1", "fileId":"file-1", "method":"PUT",
            "url":"https://oss.test/media/file-1?signature=short-lived",
            "stablePath":"/oss/media/object/file-1",
            "objectKey":"media/file-1",
            "headers":{"Content-Type":"application/pdf","x-oss-forbid-overwrite":"true"}
        })),
    );
    h.step(prepare_reply, 1002);
    let put_persist = persist_corr_for_upsert_stage(&h, "put");
    let put_row = h
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Persist { ops, .. } => ops.iter().find_map(|op| match op {
                StorageOp::BatchUpsert(spec) if spec.table == "pending_media" => spec.rows.first(),
                _ => None,
            }),
            _ => None,
        })
        .expect("put journal row");
    assert!(put_row.iter().any(|(column, value)| column == "upload_url"
        && matches!(value, SqlValue::Text(value) if value.is_empty())));
    assert!(put_row
        .iter()
        .any(|(column, value)| column == "headers_json"
            && matches!(value, SqlValue::Text(value) if value == "[]")));
    assert_eq!(upload_count(&h), 0);
    h.step(port_reply_err(put_persist), 1003);
    assert_eq!(upload_count(&h), 0);
    assert!(http(&h, "/oss/completeMediaUpload").is_empty());

    let mut h = Harness::new(&[1]);
    h.step(file_command(), 2000);
    let initial = persist_corr_for_upsert_stage(&h, "prepare");
    h.step(port_reply_ok(initial), 2001);
    let prepare = http(&h, "/oss/getPresignedUrl")[0].0;
    h.step(
        port_reply_with_body(
            prepare,
            prepare_success(json!({
                "isSuccess":true, "uploadToken":"upload-1", "fileId":"file-1", "method":"PUT",
                "url":"https://oss.test/media/file-1?signature=short-lived",
                "stablePath":"/oss/media/object/file-1",
                "objectKey":"media/file-1",
                "headers":{"Content-Type":"application/pdf","x-oss-forbid-overwrite":"true"}
            })),
        ),
        2002,
    );
    let put_persist = persist_corr_for_upsert_stage(&h, "put");
    h.step(port_reply_ok(put_persist), 2003);
    let upload = h
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::UploadFile { corr, .. } => Some(corr.raw()),
            _ => None,
        })
        .unwrap();
    h.step(port_reply_ok(upload), 2004);
    let complete_persist = persist_corr_for_upsert_stage(&h, "complete");
    assert!(http(&h, "/oss/completeMediaUpload").is_empty());
    h.step(port_reply_err(complete_persist), 2005);
    assert!(http(&h, "/oss/completeMediaUpload").is_empty());
}

#[test]
fn restart_from_prepare_retries_prepare_with_same_temporary_id() {
    let row = pending_row(
        TMP,
        "file",
        "prepare",
        "doc.pdf",
        "application/pdf",
        9,
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "",
        "",
    );
    let mut h = restart_with(vec![row]);
    retry_with_message(
        &mut h,
        TMP,
        message_row(
            TMP,
            "file",
            json!({"file":{"upload":{"status":"preparing"}}}),
            "sending",
        ),
    );

    assert_eq!(http(&h, "/oss/getPresignedUrl").len(), 1);
    assert_eq!(upload_count(&h), 0);
    assert!(http(&h, "/posts/create").is_empty());
}

#[test]
fn restart_from_put_retries_prepare_without_reusing_persisted_credentials() {
    let row = pending_row(
        TMP,
        "file",
        "put",
        "doc.pdf",
        "application/pdf",
        9,
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "upload-1",
        "file-1",
    );
    let mut h = restart_with(vec![row]);
    retry_with_message(
        &mut h,
        TMP,
        message_row(
            TMP,
            "file",
            json!({"file":{"upload":{"status":"uploading"}}}),
            "sending",
        ),
    );

    assert_eq!(http(&h, "/oss/getPresignedUrl").len(), 1);
    assert_eq!(upload_count(&h), 0);
    assert!(http(&h, "/oss/completeMediaUpload").is_empty());
}

#[test]
fn restart_from_complete_retries_complete_without_repeating_prepare_or_put() {
    let row = pending_row(
        TMP,
        "file",
        "complete",
        "doc.pdf",
        "application/pdf",
        9,
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "upload-1",
        "file-1",
    );
    let mut h = restart_with(vec![row]);
    retry_with_message(
        &mut h,
        TMP,
        message_row(
            TMP,
            "file",
            json!({"file":{"upload":{"status":"completing"}}}),
            "sending",
        ),
    );

    assert!(http(&h, "/oss/getPresignedUrl").is_empty());
    assert_eq!(upload_count(&h), 0);
    assert_eq!(http(&h, "/oss/completeMediaUpload").len(), 1);
}

#[test]
fn duplicate_retry_is_idempotent_while_recovered_stage_is_inflight() {
    let row = pending_row(
        TMP,
        "file",
        "complete",
        "doc.pdf",
        "application/pdf",
        9,
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "upload-1",
        "file-1",
    );
    let mut h = restart_with(vec![row]);
    retry_with_message(
        &mut h,
        TMP,
        message_row(
            TMP,
            "file",
            json!({"file":{"upload":{"status":"completing"}}}),
            "sending",
        ),
    );
    let duplicate = h.step(
        command_json("im_retry_send", json!({"message_id": TMP})),
        12,
    );

    assert!(duplicate.is_empty());
}

#[test]
fn corrupted_durable_row_fails_the_whole_recovery_scan_without_starting_io() {
    let row = pending_row(
        TMP,
        "file",
        "unknown",
        "doc.pdf",
        "application/pdf",
        9,
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "upload-1",
        "file-1",
    );
    let mut h = Harness::new(&[1]);
    h.start(0);
    let corr = pending_scan_corr(&h);
    let tick = port_reply_with_body(corr, Bytes::from(serde_json::to_vec(&vec![row]).unwrap()));
    use helix_core::Module;
    h.sink.clear();
    let result = h.module.handle(&tick, 1, &mut h.sink);

    assert!(result.is_err(), "malformed journal scan must fail closed");
    assert_eq!(upload_count(&h), 0);
    assert!(http(&h, "/oss/getPresignedUrl").is_empty());
    assert!(http(&h, "/oss/completeMediaUpload").is_empty());
    assert!(http(&h, "/posts/create").is_empty());
    assert!(h.sink.as_slice().is_empty());
}

#[test]
fn multi_attachment_partial_restart_completes_remaining_barrier_and_posts_once() {
    let row = pending_row(
        "tmp-durable-rich",
        "rich:1",
        "complete",
        "b.png",
        "image/png",
        4,
        "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
        "upload-b",
        "file-b",
    );
    let props = json!({
        "imagePosition": "bottom",
        "files": [
            {"id":"file-a","name":"a.png","url":"http://localhost:3399/oss/media/object/file-a","uri":"http://localhost:3399/oss/media/object/file-a","contentType":"image/png","size":3,"sha256":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","upload":{"status":"completed"}},
            {"height":600,"upload":{"status":"completing"}}
        ]
    });
    let mut h = restart_with(vec![row]);
    retry_with_message(
        &mut h,
        "tmp-durable-rich",
        message_row("tmp-durable-rich", "rich", props, "sending"),
    );
    assert_eq!(http(&h, "/oss/completeMediaUpload").len(), 1);
    assert!(http(&h, "/posts/create").is_empty());

    let corr = http(&h, "/oss/completeMediaUpload")[0].0;
    h.step(
        with_corr(
            corr,
            complete_success(
                "upload-b",
                "file-b",
                "b.png",
                4,
                "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
            ),
        ),
        20,
    );
    assert!(http(&h, "/posts/create").is_empty());
    let completion = completion_commit_corr(&h);
    h.step(port_reply_ok(completion), 21);
    assert_eq!(http(&h, "/posts/create").len(), 1);
    let body: Value = serde_json::from_slice(
        http(&h, "/posts/create")[0]
            .1
            .body
            .as_ref()
            .unwrap()
            .as_ref(),
    )
    .unwrap();
    assert_eq!(body["temporaryId"], "tmp-durable-rich");
    assert_eq!(body["type"], "IMAGE");
    assert_eq!(body["props"]["imagePosition"], "bottom");
    assert!(body["props"].get("images").is_none());
    assert!(body["props"].get("textPosition").is_none());
    assert_eq!(body["props"]["files"][0]["id"], "file-a");
    assert_eq!(body["props"]["files"][1]["id"], "file-b");
}

#[test]
fn completion_deletes_internal_rows_and_second_restart_does_not_repost() {
    let row = pending_row(
        TMP,
        "file",
        "complete",
        "doc.pdf",
        "application/pdf",
        9,
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "upload-1",
        "file-1",
    );
    let mut h = restart_with(vec![row]);
    retry_with_message(
        &mut h,
        TMP,
        message_row(
            TMP,
            "file",
            json!({"file":{"upload":{"status":"completing"}}}),
            "sending",
        ),
    );
    let corr = http(&h, "/oss/completeMediaUpload")[0].0;
    h.step(
        with_corr(
            corr,
            complete_success(
                "upload-1",
                "file-1",
                "doc.pdf",
                9,
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            ),
        ),
        20,
    );
    let completion = completion_commit_corr(&h);
    assert!(h.sink.as_slice().iter().any(|effect| match effect {
        Effect::PersistAtomic { ops, .. } => ops.iter().any(|op| {
            matches!(op, StorageOp::BatchDelete(spec) if spec.table == "pending_media")
        }),
        _ => false,
    }));
    h.step(port_reply_ok(completion), 21);

    let mut second = restart_with(Vec::new());
    retry_with_message(
        &mut second,
        TMP,
        message_row(
            TMP,
            "file",
            json!({"file":{
                "bucket":"file",
                "id":"file-1",
                "name":"doc.pdf",
                "url":"http://localhost:3399/oss/media/object/file-1",
                "uri":"http://localhost:3399/oss/media/object/file-1",
                "contentType":"application/pdf",
                "size":9,
                "sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "upload":{"status":"completed"}
            }}),
            "sent",
        ),
    );
    assert!(http(&second, "/posts/create").is_empty());
    assert_eq!(upload_count(&second), 0);
}

#[test]
fn restart_after_final_completion_commit_retries_only_posts_create() {
    let completed_props = json!({"file":{
        "bucket":"file",
        "id":"file-1",
        "name":"doc.pdf",
        "url":"http://localhost:3399/oss/media/object/file-1",
        "uri":"http://localhost:3399/oss/media/object/file-1",
        "contentType":"application/pdf",
        "size":9,
        "sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "upload":{"status":"completed"}
    }});
    let mut restarted = restart_with(Vec::new());

    retry_with_message(
        &mut restarted,
        TMP,
        message_row(TMP, "file", completed_props, "unsend"),
    );

    let posts = http(&restarted, "/posts/create");
    assert_eq!(posts.len(), 1);
    let body: Value = serde_json::from_slice(posts[0].1.body.as_ref().unwrap().as_ref()).unwrap();
    assert_eq!(body["temporaryId"], TMP);
    assert!(http(&restarted, "/oss/getPresignedUrl").is_empty());
    assert!(http(&restarted, "/oss/completeMediaUpload").is_empty());
    assert_eq!(upload_count(&restarted), 0);

    let duplicate = restarted.step(
        command_json("im_retry_send", json!({"message_id": TMP})),
        13,
    );
    assert!(
        duplicate
            .iter()
            .all(|effect| !matches!(effect, common::EffectKind::Http(_))),
        "同一进程内重复 retry 不得再次发送 posts/create"
    );
}
