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
        "uploadId": media.upload_id,
        "fileId": media.file_id,
        "method": "PUT",
        "url": format!("http://test/media/upload/{}", media.upload_id),
        "publicUrl": format!("http://test/media/object/{}", media.file_id),
        "objectKey": format!("object-internal-{}", media.file_id),
        "headers": {
            "Content-Type": media.content_type,
            "Cookieid": "owner-internal",
            "companyId": "team-internal"
        }
    })
}

fn complete_data(media: MediaFixture) -> Value {
    json!({
        "id": media.file_id,
        "uploadId": media.upload_id,
        "fileId": media.file_id,
        "url": format!("http://test/media/object/{}", media.file_id),
        "size": media.size,
        "contentType": media.content_type,
        "fileName": media.file_name,
        "sha256": media.sha256,
        "channelId": cid(1),
        "userId": "owner-internal",
        "teamId": "team-internal",
        "etag": "etag-internal",
        "state": "completed"
    })
}

fn success(data: Value) -> Tick {
    let common_res =
        Bytes::from(serde_json::to_vec(&json!({"status": "SUCCESS", "data": data})).unwrap());
    port_reply_with_body(0, enveloped(common_res))
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

fn put_one(h: &mut Harness, corr: u64, now_ms: u64) -> u64 {
    h.step(port_reply_ok(corr), now_ms);
    let persist = pending_stage_corr(h, "complete");
    h.step(port_reply_ok(persist), now_ms + 1);
    http_effects(h, "/media/upload/complete")[0].0
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
    let prepare_corr = http_effects(&h, "/media/upload/prepare")[0].0;
    let upload = prepare_one(&mut h, prepare_corr, FILE, 1010);
    let complete = put_one(&mut h, upload, 1020);

    let effects = h.step(reply(complete, success(complete_data(FILE))), 1030);

    assert_eq!(posts_count(&h), 1);
    assert!(h.sink.as_slice().iter().any(|effect| matches!(
        effect,
        Effect::Persist { ops, .. }
            if ops.iter().any(|op| matches!(
                op,
                StorageOp::BatchUpdate(spec)
                    if spec.table == "message"
                        && spec.patch.iter().any(|(column, value)|
                            column == "upload_progress_percent"
                                && matches!(value, SqlValue::Integer(100)))
            ))
    )));
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
    assert_eq!(body["props"]["file"]["id"], FILE.file_id);
    assert_eq!(
        body["props"]["file"]["url"],
        "http://test/media/object/file-file"
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
    let prepares = http_effects(&h, "/media/upload/prepare");
    assert_eq!(prepares.len(), 2);
    let upload_a = prepare_one(&mut h, prepares[0].0, IMAGE_A, 1010);
    let upload_b = prepare_one(&mut h, prepares[1].0, IMAGE_B, 1020);
    let complete_b = put_one(&mut h, upload_b, 1030);
    let complete_a = put_one(&mut h, upload_a, 1040);

    let first = h.step(reply(complete_b, success(complete_data(IMAGE_B))), 1050);
    assert_eq!(posts_count(&h), 0);
    assert!(first
        .iter()
        .all(|effect| !matches!(effect, EffectKind::ScheduleTimer(_))));
    let duplicate_first = h.step(reply(complete_b, success(complete_data(IMAGE_B))), 1060);
    assert!(duplicate_first.is_empty());

    let final_effects = h.step(reply(complete_a, success(complete_data(IMAGE_A))), 1070);
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
fn one_mismatched_attachment_closes_the_multi_attachment_gate() {
    let mut h = Harness::new(&[1]);
    h.step(rich_cmd(), 1000);
    release_initial(&mut h, 1001);
    let prepares = http_effects(&h, "/media/upload/prepare");
    let upload_a = prepare_one(&mut h, prepares[0].0, IMAGE_A, 1010);
    let upload_b = prepare_one(&mut h, prepares[1].0, IMAGE_B, 1020);
    let complete_a = put_one(&mut h, upload_a, 1030);
    let complete_b = put_one(&mut h, upload_b, 1040);

    h.step(reply(complete_a, success(complete_data(IMAGE_A))), 1050);
    assert_eq!(posts_count(&h), 0);
    let mut mismatch = complete_data(IMAGE_B);
    mismatch["sha256"] = json!("wrong-sha");
    let failure_effects = h.step(reply(complete_b, success(mismatch)), 1060);

    assert_eq!(posts_count(&h), 0);
    assert!(failure_effects
        .iter()
        .all(|effect| !matches!(effect, EffectKind::ScheduleTimer(_))));
}
