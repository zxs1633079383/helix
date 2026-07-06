mod common;

use bytes::Bytes;
use common::{
    cid, command_json, port_reply_ok, port_reply_with_body, send_message_cmd_json, Harness,
};
use helix_core::effect::{Effect, SqlValue, StorageOp, UpsertSpec};
use helix_core::{EffectSink, Module, Tick};
use helix_im::state::SendStatus;
use serde_json::{json, Value};

fn rich_two_image_cmd() -> Tick {
    send_message_cmd_json(json!({
        "channel_id": cid(1),
        "temporary_id": "tmp-rich-1",
        "type": "rich",
        "text": "hello",
        "props": {
            "textPosition": "bottom",
            "images": [
                {
                    "localPath": "/tmp/a.png",
                    "width": 800,
                    "upload": {
                        "method": "PUT",
                        "url": "https://upload/a",
                        "headers": {"x-oss-a": "1"},
                        "objectKey": "im/a.png",
                        "contentType": "image/png",
                        "size": 3
                    }
                },
                {
                    "localPath": "/tmp/b.png",
                    "height": 600,
                    "upload": {
                        "method": "PUT",
                        "url": "https://upload/b",
                        "headers": {"x-oss-b": "2"},
                        "objectKey": "im/b.png"
                    }
                }
            ],
            "frontendCustom": {"keep": true}
        }
    }))
}

fn rich_single_image_cmd(tmp: &str) -> Tick {
    send_message_cmd_json(json!({
        "channel_id": cid(1),
        "temporary_id": tmp,
        "type": "rich",
        "text": "hello",
        "props": {
            "textPosition": "bottom",
            "images": [{
                "localPath": "/tmp/a.png",
                "upload": {
                    "method": "PUT",
                    "url": "https://upload/a",
                    "headers": {},
                    "objectKey": "im/a.png"
                }
            }],
            "frontendCustom": {"keep": true}
        }
    }))
}

fn file_cmd(tmp: &str) -> Tick {
    send_message_cmd_json(json!({
        "channel_id": cid(1),
        "temporary_id": tmp,
        "type": "file",
        "text": "",
        "props": {
            "file": {
                "localPath": "/tmp/doc.pdf",
                "name": "doc.pdf",
                "upload": {
                    "method": "PUT",
                    "url": "https://upload/doc",
                    "headers": {"x-oss-doc": "1"},
                    "objectKey": "im/doc.pdf",
                    "contentType": "application/pdf",
                    "size": 9
                }
            },
            "frontendCustom": {"keep": true}
        }
    }))
}

fn upload_reply(object_key: &str, url: &str, etag: &str) -> Tick {
    port_reply_with_body(
        0,
        Bytes::from(
            serde_json::to_vec(&json!({
                "objectKey": object_key,
                "url": url,
                "etag": etag
            }))
            .expect("upload reply json"),
        ),
    )
}

fn upload_effect_corrs(h: &Harness) -> Vec<u64> {
    h.sink
        .as_slice()
        .iter()
        .filter_map(|effect| match effect {
            Effect::UploadFile { corr, .. } => Some(corr.raw()),
            _ => None,
        })
        .collect()
}

fn last_upload_request(h: &Harness) -> helix_core::effect::FileUploadRequest {
    h.sink
        .as_slice()
        .iter()
        .rev()
        .find_map(|effect| match effect {
            Effect::UploadFile { req, .. } => Some(req.clone()),
            _ => None,
        })
        .expect("upload request")
}

fn persist_corr(h: &Harness) -> u64 {
    h.sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Persist { corr, .. } => Some(corr.raw()),
            _ => None,
        })
        .expect("persist corr")
}

fn send_timer_ids(h: &Harness) -> Vec<u64> {
    h.sink
        .as_slice()
        .iter()
        .filter_map(|effect| match effect {
            Effect::ScheduleTimer { id, .. } => Some(id.raw()),
            _ => None,
        })
        .collect()
}

fn posts_create_count(h: &Harness) -> usize {
    h.sink
        .as_slice()
        .iter()
        .filter(|effect| match effect {
            Effect::Http { req, .. } => req.url.ends_with("/posts/create"),
            _ => false,
        })
        .count()
}

fn last_posts_create_body(h: &Harness) -> Value {
    h.sink
        .as_slice()
        .iter()
        .rev()
        .find_map(|effect| match effect {
            Effect::Http { req, .. } if req.url.ends_with("/posts/create") => req
                .body
                .as_ref()
                .and_then(|body| serde_json::from_slice(body.as_ref()).ok()),
            _ => None,
        })
        .expect("posts/create body")
}

fn last_message_upsert(h: &Harness) -> UpsertSpec {
    h.sink
        .as_slice()
        .iter()
        .rev()
        .find_map(|effect| match effect {
            Effect::Persist { ops, .. } | Effect::PersistFire { ops } => {
                ops.iter().find_map(|op| match op {
                    StorageOp::BatchUpsert(spec) if spec.table == "message" => Some(spec.clone()),
                    _ => None,
                })
            }
            _ => None,
        })
        .expect("message upsert")
}

fn row_text<'a>(row: &'a [(String, SqlValue)], key: &str) -> Option<&'a str> {
    row.iter()
        .find_map(|(column, value)| match (column.as_str(), value) {
            (c, SqlValue::Text(text)) if c == key => Some(text.as_str()),
            _ => None,
        })
}

fn props_from_spec(spec: &UpsertSpec) -> Value {
    let row = spec.rows.first().expect("message row");
    serde_json::from_str(row_text(row, "props").expect("props column")).expect("props json")
}

fn assert_no_unsend_persist_fire(h: &Harness) {
    assert!(
        h.sink.as_slice().iter().all(|effect| {
            !matches!(
                effect,
                Effect::PersistFire { ops }
                    if ops.iter().any(|op| match op {
                        StorageOp::BatchUpsert(spec) if spec.table == "message" => spec.rows.iter().any(|row| {
                            row_text(row, "send_status") == Some("unsend")
                        }),
                        _ => false,
                    })
            )
        }),
        "must not persist send_status=unsend"
    );
}

#[test]
fn rich_two_images_initial_step_persists_uploading_props_and_defers_posts_create() {
    let mut h = Harness::new(&[1]);

    let _effects = h.step(rich_two_image_cmd(), 1000);

    assert_eq!(
        upload_effect_corrs(&h).len(),
        2,
        "should emit two UploadFile effects"
    );
    assert_eq!(
        posts_create_count(&h),
        0,
        "posts/create must wait for uploads"
    );
    assert!(
        send_timer_ids(&h).is_empty(),
        "upload-gated initial step must not arm post timer"
    );

    let spec = last_message_upsert(&h);
    let row = spec.rows.first().expect("message row");
    assert_eq!(row_text(row, "type"), Some("rich"));
    let props = props_from_spec(&spec);
    assert_eq!(props["images"][0]["upload"]["status"], "uploading");
    assert_eq!(props["images"][1]["upload"]["status"], "uploading");
    assert_eq!(props["frontendCustom"]["keep"], true);
}

#[test]
fn rich_uploads_only_post_after_all_success() {
    let mut h = Harness::new(&[1]);

    h.step(rich_two_image_cmd(), 1000);
    let p1 = persist_corr(&h);
    let upload_corrs = upload_effect_corrs(&h);
    assert_eq!(upload_corrs.len(), 2, "need two upload corrs");

    h.step(port_reply_ok(p1), 1010);
    assert_eq!(
        h.module.pending_send_status("tmp-rich-1"),
        Some(SendStatus::Sending),
        "p1 ok should still advance send status"
    );

    let first_ok = Tick::PortReply {
        corr: helix_core::Correlation::from_raw(upload_corrs[0]),
        outcome: match upload_reply("im/a.png", "https://cdn/a.png", "etag-a") {
            Tick::PortReply { outcome, .. } => outcome,
            _ => unreachable!(),
        },
    };
    h.step(first_ok, 1020);
    assert_eq!(
        posts_create_count(&h),
        0,
        "one upload success is not enough"
    );
    let first_props = props_from_spec(&last_message_upsert(&h));
    assert_eq!(first_props["images"][0]["upload"]["status"], "success");
    assert_eq!(first_props["images"][1]["upload"]["status"], "uploading");
    assert_eq!(first_props["frontendCustom"]["keep"], true);

    let second_ok = Tick::PortReply {
        corr: helix_core::Correlation::from_raw(upload_corrs[1]),
        outcome: match upload_reply("im/b.png", "https://cdn/b.png", "etag-b") {
            Tick::PortReply { outcome, .. } => outcome,
            _ => unreachable!(),
        },
    };
    let final_effects = h.step(second_ok, 1030);

    assert_eq!(
        posts_create_count(&h),
        1,
        "all uploads success should emit one posts/create"
    );
    assert!(
        final_effects
            .iter()
            .any(|effect| matches!(effect, common::EffectKind::ScheduleTimer(_))),
        "last upload success should arm post timer in the same step"
    );
    let body = last_posts_create_body(&h);
    assert_eq!(body["props"]["images"][0]["upload"]["status"], "success");
    assert_eq!(body["props"]["images"][1]["upload"]["status"], "success");
    assert_eq!(body["props"]["images"][0]["width"], 800);
    assert_eq!(body["props"]["images"][1]["height"], 600);
    assert_eq!(body["props"]["frontendCustom"]["keep"], true);
}

#[test]
fn upload_failure_only_marks_target_failed_without_posts_create_or_unsend() {
    let mut h = Harness::new(&[1]);

    h.step(rich_single_image_cmd("tmp-rich-fail"), 1000);
    let p1 = persist_corr(&h);
    let upload_corr = *upload_effect_corrs(&h).first().expect("upload corr");

    h.step(port_reply_ok(p1), 1010);
    assert_eq!(
        h.module.pending_send_status("tmp-rich-fail"),
        Some(SendStatus::Sending)
    );

    h.step(common::port_reply_err(upload_corr), 1020);

    assert_eq!(
        posts_create_count(&h),
        0,
        "upload failure must not send posts/create"
    );
    let props = props_from_spec(&last_message_upsert(&h));
    assert_eq!(props["images"][0]["upload"]["status"], "failed");
    assert_eq!(props["frontendCustom"]["keep"], true);
    assert_eq!(
        h.module.pending_send_status("tmp-rich-fail"),
        Some(SendStatus::Sending),
        "upload failure must not map to UnSend"
    );
}

#[test]
fn retry_failed_rich_image_emits_one_upload_without_posts_create() {
    let mut h = Harness::new(&[1]);

    h.step(rich_two_image_cmd(), 1000);
    let p1 = persist_corr(&h);
    let upload_corrs = upload_effect_corrs(&h);

    h.step(port_reply_ok(p1), 1010);
    h.step(
        Tick::PortReply {
            corr: helix_core::Correlation::from_raw(upload_corrs[0]),
            outcome: match upload_reply("im/a.png", "https://cdn/a.png", "etag-a") {
                Tick::PortReply { outcome, .. } => outcome,
                _ => unreachable!(),
            },
        },
        1020,
    );
    h.step(common::port_reply_err(upload_corrs[1]), 1030);

    let effects = h.step(
        command_json(
            "im_retry_upload",
            json!({
                "temporary_id": "tmp-rich-1",
                "target": {"kind": "rich_image", "index": 1}
            }),
        ),
        1040,
    );

    assert_eq!(
        effects
            .iter()
            .filter(|effect| matches!(effect, common::EffectKind::UploadFile(_)))
            .count(),
        1,
        "retry must emit exactly one UploadFile"
    );
    assert_eq!(
        posts_create_count(&h),
        0,
        "retry must not emit posts/create"
    );
    let req = last_upload_request(&h);
    assert_eq!(req.local_path, "/tmp/b.png");
    assert_eq!(req.object_key, "im/b.png");
    assert_eq!(req.url, "https://upload/b");
    let retry_props = props_from_spec(&last_message_upsert(&h));
    assert_eq!(retry_props["images"][0]["upload"]["status"], "success");
    assert_eq!(retry_props["images"][1]["upload"]["status"], "uploading");
    assert_eq!(retry_props["frontendCustom"]["keep"], true);

    let retry_corr = *upload_effect_corrs(&h).last().expect("retry upload corr");
    let final_effects = h.step(
        Tick::PortReply {
            corr: helix_core::Correlation::from_raw(retry_corr),
            outcome: match upload_reply("im/b.png", "https://cdn/b.png", "etag-b") {
                Tick::PortReply { outcome, .. } => outcome,
                _ => unreachable!(),
            },
        },
        1050,
    );

    assert_eq!(
        posts_create_count(&h),
        1,
        "retry success should reuse upload reply flow and emit posts/create"
    );
    assert!(
        final_effects
            .iter()
            .any(|effect| matches!(effect, common::EffectKind::ScheduleTimer(_))),
        "retry success should re-arm post timer with posts/create"
    );
    let body = last_posts_create_body(&h);
    assert_eq!(body["props"]["images"][0]["upload"]["status"], "success");
    assert_eq!(body["props"]["images"][1]["upload"]["status"], "success");
}

#[test]
fn retry_failed_file_emits_one_upload_without_posts_create() {
    let mut h = Harness::new(&[1]);

    h.step(file_cmd("tmp-file-retry"), 1000);
    let p1 = persist_corr(&h);
    let upload_corr = *upload_effect_corrs(&h).first().expect("upload corr");

    h.step(port_reply_ok(p1), 1010);
    h.step(common::port_reply_err(upload_corr), 1020);

    let effects = h.step(
        command_json(
            "im_retry_upload",
            json!({
                "temporary_id": "tmp-file-retry",
                "target": {"kind": "file"}
            }),
        ),
        1030,
    );

    assert_eq!(
        effects
            .iter()
            .filter(|effect| matches!(effect, common::EffectKind::UploadFile(_)))
            .count(),
        1,
        "retry must emit exactly one UploadFile"
    );
    assert_eq!(
        posts_create_count(&h),
        0,
        "retry must not emit posts/create"
    );
    let req = last_upload_request(&h);
    assert_eq!(req.local_path, "/tmp/doc.pdf");
    assert_eq!(req.object_key, "im/doc.pdf");
    assert_eq!(req.url, "https://upload/doc");
    let retry_props = props_from_spec(&last_message_upsert(&h));
    assert_eq!(retry_props["file"]["upload"]["status"], "uploading");
    assert_eq!(retry_props["frontendCustom"]["keep"], true);

    let retry_corr = *upload_effect_corrs(&h).last().expect("retry upload corr");
    let final_effects = h.step(
        Tick::PortReply {
            corr: helix_core::Correlation::from_raw(retry_corr),
            outcome: match upload_reply("im/doc.pdf", "https://cdn/doc.pdf", "etag-doc-retry") {
                Tick::PortReply { outcome, .. } => outcome,
                _ => unreachable!(),
            },
        },
        1040,
    );

    assert_eq!(
        posts_create_count(&h),
        1,
        "retry success should reuse upload reply flow and emit posts/create"
    );
    assert!(
        final_effects
            .iter()
            .any(|effect| matches!(effect, common::EffectKind::ScheduleTimer(_))),
        "retry success should re-arm post timer with posts/create"
    );
    let body = last_posts_create_body(&h);
    assert_eq!(body["props"]["file"]["upload"]["status"], "success");
    assert_eq!(body["props"]["file"]["upload"]["etag"], "etag-doc-retry");
}

#[test]
fn retry_failed_target_requires_failed_status() {
    let mut h = Harness::new(&[1]);

    h.step(rich_two_image_cmd(), 1000);
    let p1 = persist_corr(&h);
    let upload_corrs = upload_effect_corrs(&h);

    h.step(port_reply_ok(p1), 1010);
    h.step(
        Tick::PortReply {
            corr: helix_core::Correlation::from_raw(upload_corrs[0]),
            outcome: match upload_reply("im/a.png", "https://cdn/a.png", "etag-a") {
                Tick::PortReply { outcome, .. } => outcome,
                _ => unreachable!(),
            },
        },
        1020,
    );
    h.step(common::port_reply_err(upload_corrs[1]), 1030);

    let retry_success_target = command_json(
        "im_retry_upload",
        json!({
            "temporary_id": "tmp-rich-1",
            "target": {"kind": "rich_image", "index": 0}
        }),
    );
    let mut out = EffectSink::new();
    let err = h
        .module
        .handle(&retry_success_target, 1040, &mut out)
        .expect_err("retrying success target must error");

    assert!(
        format!("{err}").contains("upload retry target is not failed"),
        "unexpected error: {err}"
    );
    assert!(
        out.as_slice().is_empty(),
        "error path must not emit effects"
    );
}

#[test]
fn upload_failure_send_timer_does_not_mark_post_unsend() {
    let mut h = Harness::new(&[1]);

    h.step(rich_single_image_cmd("tmp-rich-fail-timer"), 1000);
    let p1 = persist_corr(&h);
    let upload_corr = *upload_effect_corrs(&h).first().expect("upload corr");

    h.step(port_reply_ok(p1), 1010);
    h.step(common::port_reply_err(upload_corr), 1020);

    assert_eq!(
        h.module.pending_send_status("tmp-rich-fail-timer"),
        Some(SendStatus::Sending),
        "upload failure must remain separate from post send timeout"
    );
    assert!(
        send_timer_ids(&h).is_empty(),
        "upload phase must not arm post timer"
    );
    assert_no_unsend_persist_fire(&h);
}

#[test]
fn file_initial_step_uploads_then_posts_create_after_success_without_reply_object_key() {
    let mut h = Harness::new(&[1]);

    h.step(file_cmd("tmp-file-ok"), 1000);
    let p1 = persist_corr(&h);
    let upload_corr = *upload_effect_corrs(&h).first().expect("upload corr");

    assert_eq!(upload_effect_corrs(&h).len(), 1, "file should upload once");
    assert_eq!(posts_create_count(&h), 0, "file should defer posts/create");
    let initial_spec = last_message_upsert(&h);
    let initial_row = initial_spec.rows.first().expect("message row");
    assert_eq!(row_text(initial_row, "type"), Some("file"));
    let initial_props = props_from_spec(&initial_spec);
    assert_eq!(initial_props["file"]["upload"]["status"], "uploading");
    assert_eq!(initial_props["frontendCustom"]["keep"], true);

    h.step(port_reply_ok(p1), 1010);
    h.step(
        Tick::PortReply {
            corr: helix_core::Correlation::from_raw(upload_corr),
            outcome: match port_reply_with_body(
                0,
                Bytes::from(
                    serde_json::to_vec(&json!({
                        "url": "https://cdn/doc.pdf",
                        "etag": "etag-doc"
                    }))
                    .expect("upload reply json"),
                ),
            ) {
                Tick::PortReply { outcome, .. } => outcome,
                _ => unreachable!(),
            },
        },
        1020,
    );

    let success_props = props_from_spec(&last_message_upsert(&h));
    assert_eq!(success_props["file"]["upload"]["status"], "success");
    assert_eq!(success_props["file"]["upload"]["objectKey"], "im/doc.pdf");
    assert_eq!(
        posts_create_count(&h),
        1,
        "success should emit posts/create"
    );
    let body = last_posts_create_body(&h);
    assert_eq!(body["type"], "file");
    assert_eq!(body["props"]["file"]["upload"]["status"], "success");
    assert_eq!(body["props"]["file"]["upload"]["objectKey"], "im/doc.pdf");
    assert_eq!(body["props"]["file"]["upload"]["etag"], "etag-doc");
    assert_eq!(body["props"]["frontendCustom"]["keep"], true);
}

#[test]
fn file_upload_failures_only_mark_failed_without_posts_create_or_unsend() {
    let cases = [
        (
            "tmp-file-bad-reply",
            true,
            Tick::PortReply {
                corr: helix_core::Correlation::from_raw(0),
                outcome: match port_reply_with_body(0, Bytes::from_static(b"{bad json")) {
                    Tick::PortReply { outcome, .. } => outcome,
                    _ => unreachable!(),
                },
            },
        ),
        ("tmp-file-err", false, common::port_reply_err(0)),
    ];

    for (tmp_id, malformed, mut failure_tick) in cases {
        let mut h = Harness::new(&[1]);

        h.step(file_cmd(tmp_id), 1000);
        let p1 = persist_corr(&h);
        let upload_corr = *upload_effect_corrs(&h).first().expect("upload corr");

        h.step(port_reply_ok(p1), 1010);
        if let Tick::PortReply { corr, .. } = &mut failure_tick {
            *corr = helix_core::Correlation::from_raw(upload_corr);
        }
        h.step(failure_tick, 1020);

        assert_eq!(
            posts_create_count(&h),
            0,
            "failure must not send posts/create: {tmp_id}"
        );
        let props = props_from_spec(&last_message_upsert(&h));
        assert_eq!(props["file"]["upload"]["status"], "failed");
        assert_eq!(props["file"]["upload"]["objectKey"], "im/doc.pdf");
        assert_eq!(props["frontendCustom"]["keep"], true);
        assert_eq!(
            h.module.pending_send_status(tmp_id),
            Some(SendStatus::Sending),
            "failure must not map to UnSend: {tmp_id}"
        );
        assert!(
            send_timer_ids(&h).is_empty(),
            "upload-gated failure path must not arm post timer: {tmp_id}"
        );
        assert_no_unsend_persist_fire(&h);
        if malformed {
            assert_eq!(props["file"]["upload"]["status"], "failed");
        }
    }
}
