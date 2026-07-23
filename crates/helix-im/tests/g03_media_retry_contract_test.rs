mod common;

use bytes::Bytes;
use common::{
    cid, command_json, enveloped, port_reply_ok, port_reply_with_body, send_message_cmd_json,
    Harness,
};
use helix_core::effect::{Effect, HttpRequest, SqlValue, StorageOp};
use helix_core::Tick;
use helix_im::state::SendStatus;
use serde_json::{json, Value};

#[derive(Clone, Copy)]
struct Media {
    upload: &'static str,
    file: &'static str,
    name: &'static str,
    sha: &'static str,
}

const FILE: Media = Media {
    upload: "upload-file",
    file: "file-file",
    name: "doc.pdf",
    sha: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
};
const A: Media = Media {
    upload: "upload-a",
    file: "file-a",
    name: "a.png",
    sha: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
};
const B: Media = Media {
    upload: "upload-b",
    file: "file-b",
    name: "b.png",
    sha: "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
};

fn file_cmd() -> Tick {
    send_message_cmd_json(json!({
        "channel_id": cid(1),
        "temporary_id": "tmp-retry-file",
        "type": "file",
        "text": "",
        "props": {"file": {"mediaInput": media_input(FILE)}}
    }))
}

fn rich_cmd() -> Tick {
    send_message_cmd_json(json!({
        "channel_id": cid(1),
        "temporary_id": "tmp-retry-rich",
        "type": "rich",
        "text": "hello",
        "props": {
            "imagePosition": "bottom",
            "files": [
                {"mediaInput": media_input(A)},
                {"mediaInput": media_input(B)}
            ]
        }
    }))
}

fn media_input(media: Media) -> Value {
    let content_type = if media.name.ends_with(".png") {
        "image/png"
    } else {
        "application/pdf"
    };
    json!({
        "localPath": format!("/tmp/{}", media.name),
        "fileName": media.name,
        "contentType": content_type,
        "size": if media.name.ends_with(".png") { 3 } else { 9 },
        "sha256": media.sha
    })
}

fn prepare_data(media: Media) -> Value {
    let input = media_input(media);
    json!({
        "isSuccess": true,
        "uploadToken": media.upload,
        "fileId": media.file,
        "method": "PUT",
        "url": format!("https://oss.test/media/{}?signature={}", media.file, media.upload),
        "stablePath": format!("/oss/media/object/{}", media.file),
        "objectKey": format!("media/{}", media.file),
        "headers": {
            "Content-Type": input["contentType"],
            "x-oss-forbid-overwrite": "true"
        }
    })
}

fn complete_data(media: Media) -> Value {
    let input = media_input(media);
    json!({
        "isSuccess": true,
        "state": "completed",
        "fileId": media.file,
        "objectKey": format!("media/{}", media.file),
        "stablePath": format!("/oss/media/object/{}", media.file),
        "contentLength": input["size"],
        "contentType": input["contentType"],
        "sha256": media.sha,
        "etag": "etag-internal"
    })
}

fn common_res(data: Value) -> Tick {
    port_reply_with_body(
        0,
        enveloped(Bytes::from(serde_json::to_vec(&data).unwrap())),
    )
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

fn retry(tmp: &str) -> Tick {
    command_json("im_retry_send", json!({"message_id": tmp}))
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

fn upload_corr(h: &Harness) -> u64 {
    h.sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::UploadFile { corr, .. } => Some(corr.raw()),
            _ => None,
        })
        .expect("upload corr")
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

fn retry_reset_corr(h: &Harness) -> u64 {
    h.sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
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
        .expect("media retry reset atomic persist")
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

fn release_initial(h: &mut Harness, now: u64) {
    let corr = pending_stage_corr(h, "prepare");
    h.step(port_reply_ok(corr), now);
}

fn prepare_ok(h: &mut Harness, corr: u64, media: Media, now: u64) -> u64 {
    h.step(with_corr(corr, common_res(prepare_data(media))), now);
    let persist = pending_stage_corr(h, "put");
    h.step(port_reply_ok(persist), now + 1);
    upload_corr(h)
}

fn put_ok(h: &mut Harness, corr: u64, now: u64) -> u64 {
    h.step(port_reply_ok(corr), now);
    let persist = pending_stage_corr(h, "complete");
    h.step(port_reply_ok(persist), now + 1);
    http(h, "/oss/verifyUpload")[0].0
}

fn posts(h: &Harness) -> usize {
    http(h, "/posts/create").len()
}

fn assert_retryable_media_failure(h: &Harness, temporary_id: &str, resets_progress: bool) {
    assert_eq!(
        h.module.pending_send_status(temporary_id),
        Some(SendStatus::UnSend),
        "media failure must update the in-memory send terminal"
    );
    let row = h
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Persist { ops, .. } => ops.iter().find_map(|op| match op {
                StorageOp::BatchUpsert(spec) if spec.table == "message" => {
                    spec.rows.iter().find(|row| {
                        row.iter().any(|(column, value)| {
                            column == "temporary_id"
                                && matches!(value, SqlValue::Text(value) if value == temporary_id)
                        })
                    })
                }
                _ => None,
            }),
            _ => None,
        })
        .expect("media failure must persist the message terminal");
    assert!(row.iter().any(|(column, value)| {
        column == "send_status" && matches!(value, SqlValue::Text(value) if value == "unsend")
    }));
    assert!(row.iter().any(|(column, value)| {
        column == "props"
            && matches!(
                value,
                SqlValue::Text(value)
                    if serde_json::from_str::<Value>(value)
                        .ok()
                        .is_some_and(|props| props.to_string().contains("\"status\":\"failed\""))
            )
    }));
    assert_eq!(
        row.iter().any(|(column, value)| {
            column == "upload_progress_percent" && matches!(value, SqlValue::Integer(0))
        }),
        resets_progress
    );
    assert!(h.sink.as_slice().iter().any(|effect| match effect {
        Effect::Emit { event } => serde_json::from_slice::<Value>(event.0.as_ref())
            .ok()
            .is_some_and(|value| {
                value["event"] == "im:post:send-failed"
                    && value["data"]["temporaryId"] == temporary_id
            }),
        _ => false,
    }));
}

#[test]
fn prepare_failure_retries_prepare_without_new_business_id() {
    let mut h = Harness::new(&[1]);
    h.step(file_cmd(), 1000);
    release_initial(&mut h, 1001);
    let first_prepare = http(&h, "/oss/getPresignedUrl")[0].0;
    h.step(common::port_reply_err(first_prepare), 1010);
    assert_retryable_media_failure(&h, "tmp-retry-file", true);

    h.step(retry("tmp-retry-file"), 1020);
    assert_eq!(
        h.module.pending_send_status("tmp-retry-file"),
        Some(SendStatus::UnSend),
        "retry reset durable ack 前内存状态仍须保持失败"
    );
    assert!(http(&h, "/oss/getPresignedUrl").is_empty());
    let retry_reset = retry_reset_corr(&h);
    let duplicate = h.step(retry("tmp-retry-file"), 1021);
    assert!(duplicate.is_empty(), "retry reset 在途必须幂等");
    h.step(port_reply_ok(retry_reset), 1022);
    assert_eq!(
        h.module.pending_send_status("tmp-retry-file"),
        Some(SendStatus::Sending)
    );
    let retries = http(&h, "/oss/getPresignedUrl");
    assert_eq!(retries.len(), 1);
    assert_eq!(posts(&h), 0);
    let duplicate = h.step(retry("tmp-retry-file"), 1030);
    assert!(duplicate.is_empty(), "in-flight retry must be idempotent");

    let upload = prepare_ok(&mut h, retries[0].0, FILE, 1040);
    let complete = put_ok(&mut h, upload, 1050);
    h.step(with_corr(complete, common_res(complete_data(FILE))), 1060);
    assert_eq!(posts(&h), 0);
    let completion_commit = completion_commit_corr(&h);
    h.step(port_reply_ok(completion_commit), 1061);
    assert_eq!(posts(&h), 1);
    let body: Value = serde_json::from_slice(
        http(&h, "/posts/create")[0]
            .1
            .body
            .as_ref()
            .unwrap()
            .as_ref(),
    )
    .unwrap();
    assert_eq!(body["temporaryId"], "tmp-retry-file");
}

#[test]
fn retry_reset_persist_failure_keeps_failed_state_and_never_starts_java_io() {
    let mut h = Harness::new(&[1]);
    h.step(file_cmd(), 1000);
    release_initial(&mut h, 1001);
    let first_prepare = http(&h, "/oss/getPresignedUrl")[0].0;
    h.step(common::port_reply_err(first_prepare), 1010);

    h.step(retry("tmp-retry-file"), 1020);
    let reset = retry_reset_corr(&h);
    assert!(http(&h, "/oss/getPresignedUrl").is_empty());
    h.step(common::port_reply_err(reset), 1021);
    assert_eq!(
        h.module.pending_send_status("tmp-retry-file"),
        Some(SendStatus::UnSend)
    );
    assert!(http(&h, "/oss/getPresignedUrl").is_empty());

    h.step(retry("tmp-retry-file"), 1030);
    let _ = retry_reset_corr(&h);
    assert!(
        http(&h, "/oss/getPresignedUrl").is_empty(),
        "只有 retry reset PersistOk 才能释放 Java prepare"
    );
}

#[test]
fn put_failure_retries_from_prepare_and_never_reuses_the_old_signed_url() {
    let mut h = Harness::new(&[1]);
    h.step(file_cmd(), 1000);
    release_initial(&mut h, 1001);
    let prepare = http(&h, "/oss/getPresignedUrl")[0].0;
    let first_upload = prepare_ok(&mut h, prepare, FILE, 1010);
    h.step(common::port_reply_err(first_upload), 1020);
    assert_retryable_media_failure(&h, "tmp-retry-file", true);

    h.step(retry("tmp-retry-file"), 1030);
    assert!(http(&h, "/oss/getPresignedUrl").is_empty());
    let retry_reset = retry_reset_corr(&h);
    h.step(port_reply_ok(retry_reset), 1031);
    assert_eq!(
        h.module.pending_send_status("tmp-retry-file"),
        Some(SendStatus::Sending)
    );
    let prepares = http(&h, "/oss/getPresignedUrl");
    assert_eq!(prepares.len(), 1);
    assert!(http(&h, "/oss/verifyUpload").is_empty());
    let duplicate = h.step(retry("tmp-retry-file"), 1040);
    assert!(duplicate.is_empty());

    let retried_upload = prepare_ok(&mut h, prepares[0].0, FILE, 1050);
    let complete = put_ok(&mut h, retried_upload, 1050);
    h.step(with_corr(complete, common_res(complete_data(FILE))), 1060);
    let completion_commit = completion_commit_corr(&h);
    h.step(port_reply_ok(completion_commit), 1061);
    assert_eq!(posts(&h), 1);
}

#[test]
fn complete_failure_retries_only_complete_and_preserves_multi_barrier() {
    let mut h = Harness::new(&[1]);
    h.step(rich_cmd(), 1000);
    release_initial(&mut h, 1001);
    let prepares = http(&h, "/oss/getPresignedUrl");
    let upload_a = prepare_ok(&mut h, prepares[0].0, A, 1010);
    let upload_b = prepare_ok(&mut h, prepares[1].0, B, 1020);
    let complete_a = put_ok(&mut h, upload_a, 1030);
    h.step(port_reply_ok(upload_b), 1040);
    let complete_b_stage = pending_stage_corr(&h, "complete");
    h.step(port_reply_ok(complete_b_stage), 1041);
    assert!(
        http(&h, "/oss/verifyUpload").is_empty(),
        "第二个 verify 必须等待第一个完成原子提交"
    );
    h.step(with_corr(complete_a, common_res(complete_data(A))), 1050);
    assert_eq!(posts(&h), 0);
    let first_commit = completion_commit_corr(&h);
    h.step(port_reply_ok(first_commit), 1051);
    let complete_b = http(&h, "/oss/verifyUpload")[0].0;
    let mut mismatch = complete_data(B);
    mismatch["sha256"] = json!("wrong");
    h.step(with_corr(complete_b, common_res(mismatch)), 1060);
    assert_eq!(posts(&h), 0);
    assert_retryable_media_failure(&h, "tmp-retry-rich", false);

    h.step(retry("tmp-retry-rich"), 1070);
    assert!(http(&h, "/oss/verifyUpload").is_empty());
    let retry_reset = retry_reset_corr(&h);
    h.step(port_reply_ok(retry_reset), 1071);
    assert_eq!(
        h.module.pending_send_status("tmp-retry-rich"),
        Some(SendStatus::Sending)
    );
    assert!(h
        .sink
        .as_slice()
        .iter()
        .all(|effect| !matches!(effect, Effect::UploadFile { .. })));
    assert!(http(&h, "/oss/getPresignedUrl").is_empty());
    let retried_complete = http(&h, "/oss/verifyUpload");
    assert_eq!(retried_complete.len(), 1);
    let duplicate = h.step(retry("tmp-retry-rich"), 1080);
    assert!(duplicate.is_empty());

    h.step(
        with_corr(retried_complete[0].0, common_res(complete_data(B))),
        1090,
    );
    assert_eq!(posts(&h), 0);
    let completion_commit = completion_commit_corr(&h);
    h.step(port_reply_ok(completion_commit), 1091);
    assert_eq!(posts(&h), 1);
}
