mod common;

use bytes::Bytes;
use common::{
    cid, enveloped, port_reply_ok, port_reply_with_body, send_message_cmd_json, EffectKind, Harness,
};
use helix_core::effect::{Effect, HttpRequest, SqlValue, StorageOp};
use helix_core::Tick;
use serde_json::{json, Value};

#[derive(Clone, Copy)]
struct MediaFixture {
    upload_id: &'static str,
    file_id: &'static str,
    file_name: &'static str,
    content_type: &'static str,
    size: u64,
    sha256: &'static str,
}

const FILE: MediaFixture = MediaFixture {
    upload_id: "upload-file",
    file_id: "file-file",
    file_name: "doc.pdf",
    content_type: "application/pdf",
    size: 9,
    sha256: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
};
const IMAGE_A: MediaFixture = MediaFixture {
    upload_id: "upload-a",
    file_id: "file-a",
    file_name: "a.png",
    content_type: "image/png",
    size: 3,
    sha256: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
};
const IMAGE_B: MediaFixture = MediaFixture {
    upload_id: "upload-b",
    file_id: "file-b",
    file_name: "b.png",
    content_type: "image/png",
    size: 4,
    sha256: "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
};

fn file_cmd() -> Tick {
    send_message_cmd_json(json!({
        "channel_id": cid(1),
        "temporary_id": "tmp-post-file",
        "type": "file",
        "text": "quarterly report",
        "props": {
            "file": {"mediaInput": media_input(FILE)},
            "frontendCustom": {"keep": true}
        }
    }))
}

fn rich_cmd() -> Tick {
    send_message_cmd_json(json!({
        "channel_id": cid(1),
        "temporary_id": "tmp-post-rich",
        "type": "rich",
        "text": "hello",
        "props": {
            "imagePosition": "bottom",
            "files": [
                {"width": 800, "mediaInput": media_input(IMAGE_A)},
                {"height": 600, "mediaInput": media_input(IMAGE_B)}
            ],
            "frontendCustom": {"keep": true}
        }
    }))
}

fn nine_image_cmd() -> Tick {
    let files = (0..9)
        .map(|index| {
            json!({
                "mediaInput": {
                    "localPath": format!("/tmp/{index}.png"),
                    "fileName": format!("{index}.png"),
                    "contentType": "image/png",
                    "size": index + 1,
                    "sha256": format!("{index:064x}")
                }
            })
        })
        .collect::<Vec<_>>();
    send_message_cmd_json(json!({
        "channel_id": cid(1),
        "type": "rich",
        "text": "nine",
        "props": {
            "imagePosition": "bottom",
            "files": files
        }
    }))
}

fn media_input(media: MediaFixture) -> Value {
    json!({
        "localPath": format!("/tmp/{}", media.file_name),
        "fileName": media.file_name,
        "contentType": media.content_type,
        "size": media.size,
        "sha256": media.sha256
    })
}

fn prepare_data(media: MediaFixture) -> Value {
    json!({
        "isSuccess": true,
        "uploadToken": media.upload_id,
        "fileId": media.file_id,
        "method": "PUT",
        "url": format!("https://oss.test/media/{}?signature={}", media.file_id, media.upload_id),
        "stablePath": format!("/oss/media/object/{}", media.file_id),
        "objectKey": format!("media/{}", media.file_id),
        "headers": {
            "Content-Type": media.content_type,
            "x-oss-forbid-overwrite": "true"
        }
    })
}

fn complete_data(media: MediaFixture) -> Value {
    json!({
        "isSuccess": true,
        "state": "completed",
        "fileId": media.file_id,
        "objectKey": format!("media/{}", media.file_id),
        "stablePath": format!("/oss/media/object/{}", media.file_id),
        "contentLength": media.size,
        "contentType": media.content_type,
        "sha256": media.sha256,
        "etag": "etag-internal"
    })
}

fn success(data: Value) -> Tick {
    port_reply_with_body(
        0,
        enveloped(Bytes::from(serde_json::to_vec(&data).unwrap())),
    )
}

fn reply(corr: u64, tick: Tick) -> Tick {
    match tick {
        Tick::PortReply { outcome, .. } => Tick::PortReply {
            corr: helix_core::Correlation::from_raw(corr),
            outcome,
        },
        _ => unreachable!(),
    }
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

fn release_initial(h: &mut Harness, now_ms: u64) {
    let corr = pending_stage_corr(h, "prepare");
    h.step(port_reply_ok(corr), now_ms);
}

fn posts_count(h: &Harness) -> usize {
    http_effects(h, "/posts/create").len()
}

fn posts_body(h: &Harness) -> Value {
    let request = http_effects(h, "/posts/create")
        .pop()
        .expect("posts/create")
        .1;
    serde_json::from_slice(request.body.as_ref().unwrap().as_ref()).unwrap()
}

fn prepare_one(h: &mut Harness, corr: u64, media: MediaFixture, now_ms: u64) -> u64 {
    h.step(reply(corr, success(prepare_data(media))), now_ms);
    let persist = pending_stage_corr(h, "put");
    h.step(port_reply_ok(persist), now_ms + 1);
    upload_corr(h)
}

fn put_one(h: &mut Harness, corr: u64, now_ms: u64) -> Option<u64> {
    h.step(port_reply_ok(corr), now_ms);
    let persist = pending_stage_corr(h, "complete");
    h.step(port_reply_ok(persist), now_ms + 1);
    http_effects(h, "/oss/verifyUpload")
        .first()
        .map(|(corr, _)| *corr)
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

fn completion_writes_send_status(h: &Harness, expected: &str) -> bool {
    h.sink.as_slice().iter().any(|effect| match effect {
        Effect::PersistAtomic { ops, .. }
            if ops.iter().any(
                |op| matches!(op, StorageOp::BatchDelete(spec) if spec.table == "pending_media"),
            ) =>
        {
            ops.iter().any(|op| match op {
                StorageOp::BatchUpsert(spec) if spec.table == "message" => {
                    spec.rows.iter().flatten().any(|(column, value)| {
                        column == "send_status"
                            && matches!(value, SqlValue::Text(status) if status == expected)
                    })
                }
                _ => false,
            })
        }
        _ => false,
    })
}

fn assert_no_transient_fields(body: &Value) {
    let serialized = serde_json::to_string(body).unwrap();
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
        "owner-internal",
        "team-internal",
        "/tmp/",
    ] {
        assert!(!serialized.contains(forbidden), "leaked {forbidden}");
    }
}

#[test]
fn single_attachment_complete_posts_once_and_arms_timer() {
    let mut h = Harness::new(&[1]);
    h.step(file_cmd(), 1000);
    release_initial(&mut h, 1001);
    let prepare_request = http_effects(&h, "/oss/getPresignedUrl")[0].clone();
    assert_eq!(
        prepare_request.1.url, "http://localhost:3399/oss/getPresignedUrl",
        "媒体控制面必须走前端注入的 Java base"
    );
    let prepare_corr = prepare_request.0;
    let upload = prepare_one(&mut h, prepare_corr, FILE, 1010);
    let complete = put_one(&mut h, upload, 1020).expect("file verify request");

    h.step(reply(complete, success(complete_data(FILE))), 1030);

    assert_eq!(posts_count(&h), 0);
    assert!(
        completion_writes_send_status(&h, "unsend"),
        "最后一个媒体完成必须原子写 durable retry marker"
    );
    assert_eq!(
        h.module.pending_send_status("tmp-post-file"),
        Some(helix_im::state::SendStatus::Sending),
        "durable marker 不得把当前进程的发送态提前降为失败"
    );
    assert!(h.sink.as_slice().iter().any(|effect| matches!(
        effect,
        Effect::PersistAtomic { ops, .. }
            if ops.iter().any(|op| matches!(
                op,
                StorageOp::BatchUpsert(spec)
                    if spec.table == "message"
                        && spec.rows.iter().flatten().any(|(column, value)|
                            column == "upload_progress_percent"
                                && matches!(value, SqlValue::Integer(100)))
            ))
    )));
    let completion_commit = completion_commit_corr(&h);
    let effects = h.step(port_reply_ok(completion_commit), 1031);
    assert_eq!(posts_count(&h), 1);
    assert_eq!(
        h.module.pending_send_status("tmp-post-file"),
        Some(helix_im::state::SendStatus::Sending),
        "completion ACK 后应继续等待 Go posts/create 终态"
    );
    assert_eq!(
        http_effects(&h, "/posts/create")[0].1.url,
        "http://test/posts/create",
        "业务消息必须继续走独立 Go base"
    );
    assert_eq!(
        effects
            .iter()
            .filter(|effect| matches!(effect, EffectKind::ScheduleTimer(_)))
            .count(),
        1
    );
    let body = posts_body(&h);
    assert_eq!(body["type"], "FILE");
    assert_eq!(body["message"], "quarterly report");
    assert_eq!(body["simpleMessage"], "[文件]doc.pdf");
    assert_eq!(body["props"]["file"]["id"], FILE.file_id);
    assert_eq!(
        body["props"]["file"]["url"],
        "http://localhost:3399/oss/media/object/file-file"
    );
    assert_eq!(body["props"]["file"]["upload"]["status"], "completed");
    assert_eq!(body["props"]["frontendCustom"]["keep"], true);
    assert_no_transient_fields(&body["props"]);

    let duplicate = h.step(reply(complete, success(complete_data(FILE))), 1040);
    assert!(duplicate.is_empty());
    assert_eq!(posts_count(&h), 0);
}

#[test]
fn multi_attachment_out_of_order_waits_for_all_and_posts_exactly_once() {
    let mut h = Harness::new(&[1]);
    h.step(rich_cmd(), 1000);
    release_initial(&mut h, 1001);
    let prepares = http_effects(&h, "/oss/getPresignedUrl");
    assert_eq!(prepares.len(), 2);
    let upload_a = prepare_one(&mut h, prepares[0].0, IMAGE_A, 1010);
    let upload_b = prepare_one(&mut h, prepares[1].0, IMAGE_B, 1020);
    let complete_b = put_one(&mut h, upload_b, 1030).expect("first verify request");
    assert!(
        put_one(&mut h, upload_a, 1040).is_none(),
        "同一消息的第二个 verify 必须等待前一个完成原子提交"
    );

    let first = h.step(reply(complete_b, success(complete_data(IMAGE_B))), 1050);
    assert_eq!(posts_count(&h), 0);
    assert!(
        !completion_writes_send_status(&h, "unsend"),
        "非最后附件完成不得提前把消息标为可重试"
    );
    assert!(first
        .iter()
        .all(|effect| !matches!(effect, EffectKind::ScheduleTimer(_))));
    let first_commit = completion_commit_corr(&h);
    let duplicate_first = h.step(reply(complete_b, success(complete_data(IMAGE_B))), 1060);
    assert!(duplicate_first.is_empty());

    // 重放旧 corr 不得越过原子提交 barrier；真实提交成功后才释放下一个 verify。
    h.step(port_reply_ok(first_commit), 1061);
    let complete_a = http_effects(&h, "/oss/verifyUpload")[0].0;
    h.step(reply(complete_a, success(complete_data(IMAGE_A))), 1070);
    assert_eq!(posts_count(&h), 0);
    assert!(
        completion_writes_send_status(&h, "unsend"),
        "最后一个附件完成必须留下 ACK→posts/create 崩溃恢复标记"
    );
    let final_commit = completion_commit_corr(&h);
    let final_effects = h.step(port_reply_ok(final_commit), 1071);
    assert_eq!(posts_count(&h), 1);
    assert_eq!(
        final_effects
            .iter()
            .filter(|effect| matches!(effect, EffectKind::ScheduleTimer(_)))
            .count(),
        1
    );
    let body = posts_body(&h);
    assert_eq!(body["type"], "IMAGE");
    assert_eq!(body["simpleMessage"], "[图片]hello");
    assert_eq!(body["props"]["imagePosition"], "bottom");
    assert!(body["props"].get("images").is_none());
    assert!(body["props"].get("textPosition").is_none());
    assert_eq!(body["props"]["files"][0]["id"], IMAGE_A.file_id);
    assert_eq!(body["props"]["files"][1]["id"], IMAGE_B.file_id);
    assert_eq!(body["props"]["files"][0]["width"], 800);
    assert_eq!(body["props"]["files"][1]["height"], 600);
    assert_no_transient_fields(&body["props"]);

    let duplicate_final = h.step(reply(complete_a, success(complete_data(IMAGE_A))), 1080);
    assert!(duplicate_final.is_empty());
    assert_eq!(posts_count(&h), 0);
}

#[test]
fn nine_images_emit_prepare_requests_in_one_batch_without_progress_events() {
    let mut h = Harness::new(&[1]);
    h.step(nine_image_cmd(), 1000);
    let release_corr = pending_stage_corr(&h, "prepare");
    let effects = h.step(port_reply_ok(release_corr), 1001);

    let prepares = http_effects(&h, "/oss/getPresignedUrl");
    assert_eq!(
        effects
            .iter()
            .filter(|effect| matches!(effect, EffectKind::Http(_)))
            .count(),
        9,
        "九张图的预签名请求必须同批吐给 host 并发执行，不能串行等待上一张"
    );
    assert_eq!(prepares.len(), 9);
    let client_upload_ids = prepares
        .iter()
        .map(|(_, request)| {
            serde_json::from_slice::<Value>(request.body.as_ref().unwrap().as_ref()).unwrap()
                ["clientUploadId"]
                .as_str()
                .unwrap()
                .to_string()
        })
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(client_upload_ids.len(), 9);
    assert!(effects
        .iter()
        .all(|effect| !matches!(effect, EffectKind::Emit)));
}

#[test]
fn one_mismatched_attachment_closes_the_multi_attachment_gate() {
    let mut h = Harness::new(&[1]);
    h.step(rich_cmd(), 1000);
    release_initial(&mut h, 1001);
    let prepares = http_effects(&h, "/oss/getPresignedUrl");
    let upload_a = prepare_one(&mut h, prepares[0].0, IMAGE_A, 1010);
    let upload_b = prepare_one(&mut h, prepares[1].0, IMAGE_B, 1020);
    let complete_a = put_one(&mut h, upload_a, 1030).expect("first verify request");
    assert!(put_one(&mut h, upload_b, 1040).is_none());

    h.step(reply(complete_a, success(complete_data(IMAGE_A))), 1050);
    assert_eq!(posts_count(&h), 0);
    let first_commit = completion_commit_corr(&h);
    h.step(port_reply_ok(first_commit), 1051);
    let complete_b = http_effects(&h, "/oss/verifyUpload")[0].0;
    let mut mismatch = complete_data(IMAGE_B);
    mismatch["sha256"] = json!("wrong-sha");
    let failure_effects = h.step(reply(complete_b, success(mismatch)), 1060);

    assert_eq!(posts_count(&h), 0);
    assert!(failure_effects
        .iter()
        .all(|effect| !matches!(effect, EffectKind::ScheduleTimer(_))));
}
