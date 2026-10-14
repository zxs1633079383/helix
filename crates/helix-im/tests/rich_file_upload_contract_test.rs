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
            "imagePosition": "bottom",
            "files": [
                {
                    "localPath": "/tmp/a.png",
                    "width": 800,
                    "upload": {
                        "method": "PUT",
                        "url": "https://upload/a?signature=write-a",
                        "publicUrl": "https://cdn/a.png",
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
                        "url": "https://upload/b?signature=write-b",
                        "publicUrl": "https://cdn/b.png",
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
            "imagePosition": "bottom",
            "files": [{
                "localPath": "/tmp/a.png",
                "upload": {
                    "method": "PUT",
                    "url": "https://upload/a?signature=write-a",
                    "publicUrl": "https://cdn/a.png",
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
                    "url": "https://upload/doc?signature=write-doc",
                    "publicUrl": "https://cdn/doc.pdf",
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

fn upload_reply(object_key: &str, public_url: &str, etag: &str) -> Tick {
    port_reply_with_body(
        0,
        Bytes::from(
            serde_json::to_vec(&json!({
                "objectKey": object_key,
                "url": public_url,
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
            Effect::Persist { ops, .. }
            | Effect::PersistAtomic { ops, .. }
            | Effect::PersistFire { ops } => ops.iter().find_map(|op| match op {
                StorageOp::BatchUpsert(spec) if spec.table == "message" => Some(spec.clone()),
                _ => None,
            }),
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
#[ignore = "旧断言要求首步直接 UploadFile；新合同由 active prepare/post-create 测试覆盖"]
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
    assert_eq!(row_text(row, "type"), Some("RICH"));
    let props = props_from_spec(&spec);
    assert_eq!(props["files"][0]["upload"]["status"], "uploading");
    assert_eq!(props["files"][1]["upload"]["status"], "uploading");
    assert!(props["files"][0]["upload"].get("url").is_none());
    assert!(props["files"][0]["upload"].get("headers").is_none());
    assert_eq!(
        props["files"][0]["upload"]["publicUrl"],
        "https://cdn/a.png"
    );
    assert_eq!(props["frontendCustom"]["keep"], true);
}

#[test]
#[ignore = "旧输入要求客户端 publicUrl；新合同禁止客户端提供 server-owned publicUrl"]
fn upload_input_without_public_url_is_rejected_before_any_effect() {
    let mut h = Harness::new(&[1]);
    let command = send_message_cmd_json(json!({
        "channel_id": cid(1),
        "temporary_id": "tmp-file-missing-public-url",
        "type": "file",
        "text": "",
        "props": {
            "file": {
                "localPath": "/tmp/doc.pdf",
                "upload": {
                    "method": "PUT",
                    "url": "https://upload/doc?signature=write-secret",
                    "headers": {},
                    "objectKey": "im/doc.pdf"
                }
            }
        }
    }));
    let mut out = EffectSink::new();

    let error = h
        .module
        .handle(&command, 1000, &mut out)
        .expect_err("missing publicUrl must fail closed");

    assert!(error.to_string().contains("publicUrl"));
    assert!(
        out.as_slice().is_empty(),
        "invalid input must emit no effects"
    );
}

#[test]
fn rich_file_shape_violations_are_rejected_before_any_effect() {
    let cases = [
        (
            send_message_cmd_json(json!({
                "channel_id": cid(1),
                "temporary_id": "tmp-rich-with-file",
                "type": "rich",
                "text": "hello",
                "props": {
                    "imagePosition": "top",
                    "files": [{
                        "localPath": "/tmp/a.png",
                        "upload": {
                            "method": "PUT",
                            "url": "https://upload/a",
                            "publicUrl": "https://cdn/a.png",
                            "headers": {},
                            "objectKey": "im/a.png"
                        }
                    }],
                    "file": {"localPath": "/tmp/a.pdf"}
                }
            })),
            "rich",
        ),
        (
            send_message_cmd_json(json!({
                "channel_id": cid(1),
                "temporary_id": "tmp-file-with-text",
                "type": "file",
                "text": "not allowed",
                "props": {"file": {
                    "localPath": "/tmp/a.pdf",
                    "upload": {
                        "method": "PUT",
                        "url": "https://upload/a",
                        "publicUrl": "https://cdn/a.pdf",
                        "headers": {},
                        "objectKey": "im/a.pdf"
                    }
                }}
            })),
            "file",
        ),
    ];

    for (command, expected) in cases {
        let mut h = Harness::new(&[1]);
        let mut out = EffectSink::new();
        let error = h
            .module
            .handle(&command, 1000, &mut out)
            .expect_err("invalid rich/file shape must fail closed");
        assert!(error.to_string().contains(expected));
        assert!(out.as_slice().is_empty());
    }
}

#[test]
#[ignore = "旧测试以 UploadFile 回包直接发 post；新合同必须经过 Go complete"]
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
    assert_eq!(first_props["files"][0]["upload"]["status"], "success");
    assert_eq!(first_props["files"][1]["upload"]["status"], "uploading");
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
    assert_eq!(body["props"]["files"][0]["upload"]["status"], "success");
    assert_eq!(body["props"]["files"][1]["upload"]["status"], "success");
    assert_eq!(
        body["props"]["files"][0]["upload"]["publicUrl"],
        "https://cdn/a.png"
    );
    assert!(body["props"]["files"][0]["upload"].get("url").is_none());
    assert!(body["props"]["files"][0]["upload"].get("headers").is_none());
    assert_eq!(body["props"]["files"][0]["width"], 800);
    assert_eq!(body["props"]["files"][1]["height"], 600);
    assert_eq!(body["props"]["frontendCustom"]["keep"], true);
}

#[test]
#[ignore = "旧失败点绕过 prepare；新阶段失败关闸由 g03_media_retry_contract_test 覆盖"]
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
    assert_eq!(props["files"][0]["upload"]["status"], "failed");
    assert_eq!(props["frontendCustom"]["keep"], true);
    assert_eq!(
        h.module.pending_send_status("tmp-rich-fail"),
        Some(SendStatus::Sending),
        "upload failure must not map to UnSend"
    );
}

#[test]
#[ignore = "旧命令 im_retry_upload 已由阶段感知 im_retry_send 合同替代"]
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
    assert_eq!(req.urls.upload_url(), "https://upload/b?signature=write-b");
    assert_eq!(req.urls.public_url(), "https://cdn/b.png");
    let retry_props = props_from_spec(&last_message_upsert(&h));
    assert_eq!(retry_props["files"][0]["upload"]["status"], "success");
    assert_eq!(retry_props["files"][1]["upload"]["status"], "uploading");
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
    assert_eq!(body["props"]["files"][0]["upload"]["status"], "success");
    assert_eq!(body["props"]["files"][1]["upload"]["status"], "success");
}

#[test]
#[ignore = "旧命令 im_retry_upload 已由阶段感知 im_retry_send 合同替代"]
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
    assert_eq!(
        req.urls.upload_url(),
        "https://upload/doc?signature=write-doc"
    );
    assert_eq!(req.urls.public_url(), "https://cdn/doc.pdf");
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
#[ignore = "旧命令只校验 im_retry_upload target；新合同按 failed media stage 路由"]
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
#[ignore = "旧失败点绕过 prepare；新失败无 timer/post 由 active G03 合同覆盖"]
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
#[ignore = "旧文件链以 UploadFile 回包完成；新链要求 Prepare-PUT-Complete"]
fn file_initial_step_uploads_then_posts_create_with_public_url_only() {
    let mut h = Harness::new(&[1]);

    h.step(file_cmd("tmp-file-ok"), 1000);
    let p1 = persist_corr(&h);
    let upload_corr = *upload_effect_corrs(&h).first().expect("upload corr");

    assert_eq!(upload_effect_corrs(&h).len(), 1, "file should upload once");
    assert_eq!(posts_create_count(&h), 0, "file should defer posts/create");
    let initial_spec = last_message_upsert(&h);
    let initial_row = initial_spec.rows.first().expect("message row");
    assert_eq!(row_text(initial_row, "type"), Some("FILE"));
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
                        "objectKey": "im/doc.pdf",
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
        success_props["file"]["upload"]["publicUrl"],
        "https://cdn/doc.pdf"
    );
    assert!(success_props["file"]["upload"].get("url").is_none());
    assert!(success_props["file"]["upload"].get("headers").is_none());
    assert_eq!(
        posts_create_count(&h),
        1,
        "success should emit posts/create"
    );
    let body = last_posts_create_body(&h);
    assert_eq!(body["type"], "FILE");
    assert_eq!(body["props"]["file"]["upload"]["status"], "success");
    assert_eq!(body["props"]["file"]["upload"]["objectKey"], "im/doc.pdf");
    assert_eq!(body["props"]["file"]["upload"]["etag"], "etag-doc");
    assert_eq!(
        body["props"]["file"]["upload"]["publicUrl"],
        "https://cdn/doc.pdf"
    );
    let body_text = serde_json::to_string(&body).expect("posts.create body json");
    assert!(!body_text.contains("signature=write-doc"));
    assert_eq!(body["props"]["frontendCustom"]["keep"], true);
}

#[test]
#[ignore = "旧 UploadFile 回包 publicUrl 合同已废弃，可信 URL 只接受 complete 回包"]
fn upload_reply_without_public_url_fails_closed_without_posts_create() {
    let mut h = Harness::new(&[1]);

    h.step(file_cmd("tmp-file-no-public-url"), 1000);
    let p1 = persist_corr(&h);
    let upload_corr = *upload_effect_corrs(&h).first().expect("upload corr");
    h.step(port_reply_ok(p1), 1010);

    h.step(
        Tick::PortReply {
            corr: helix_core::Correlation::from_raw(upload_corr),
            outcome: match port_reply_with_body(
                0,
                Bytes::from_static(br#"{"objectKey":"im/doc.pdf","etag":"etag-doc"}"#),
            ) {
                Tick::PortReply { outcome, .. } => outcome,
                _ => unreachable!(),
            },
        },
        1020,
    );

    assert_eq!(posts_create_count(&h), 0);
    let props = props_from_spec(&last_message_upsert(&h));
    assert_eq!(props["file"]["upload"]["status"], "failed");
    let persisted = serde_json::to_string(&props).expect("persisted props json");
    assert!(!persisted.contains("signature=write-doc"));
}

#[test]
#[ignore = "旧文件失败用客户端 objectKey/URL；新阶段失败由 active G03 合同覆盖"]
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

mod g03_prepare_contract {
    use super::common::{
        cid, enveloped, port_reply_ok, port_reply_with_body, send_message_cmd_json, EffectKind,
        Harness,
    };
    use bytes::Bytes;
    use helix_core::effect::{
        Effect, FileUploadRequest, HttpRequest, SqlValue, StorageOp, UpsertSpec,
    };
    use helix_core::{EffectSink, Module, Tick};
    use serde_json::{json, Value};

    fn media_input(path: &str, name: &str, content_type: &str, size: u64) -> Value {
        json!({
            "localPath": path,
            "fileName": name,
            "contentType": content_type,
            "size": size,
            "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        })
    }

    fn file_cmd(tmp: &str) -> Tick {
        send_message_cmd_json(json!({
            "channel_id": cid(1),
            "temporary_id": tmp,
            "type": "file",
            "text": "",
            "props": {
                "file": {
                    "mediaInput": media_input("/tmp/doc.pdf", "doc.pdf", "application/pdf", 9)
                },
                "frontendCustom": {"keep": true}
            }
        }))
    }

    fn rich_cmd() -> Tick {
        send_message_cmd_json(json!({
            "channel_id": cid(1),
            "temporary_id": "tmp-rich",
            "type": "rich",
            "text": "hello",
            "props": {
                "imagePosition": "bottom",
                "files": [{
                    "width": 800,
                    "height": 600,
                    "mediaInput": media_input("/tmp/a.png", "a.png", "image/png", 3)
                }]
            }
        }))
    }

    fn prepare_effects(h: &Harness) -> Vec<(u64, HttpRequest)> {
        h.sink
            .as_slice()
            .iter()
            .filter_map(|effect| match effect {
                Effect::Http { corr, req } if req.url.ends_with("/oss/getPresignedUrl") => {
                    Some((corr.raw(), req.clone()))
                }
                _ => None,
            })
            .collect()
    }

    fn upload_effects(h: &Harness) -> Vec<(u64, FileUploadRequest)> {
        h.sink
            .as_slice()
            .iter()
            .filter_map(|effect| match effect {
                Effect::UploadFile { corr, req } => Some((corr.raw(), req.clone())),
                _ => None,
            })
            .collect()
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

    fn release_stage(h: &mut Harness, stage: &str, now_ms: u64) {
        let corr = pending_stage_corr(h, stage);
        h.step(port_reply_ok(corr), now_ms);
    }

    fn posts_create_count(h: &Harness) -> usize {
        h.sink
            .as_slice()
            .iter()
            .filter(|effect| {
                matches!(effect, Effect::Http { req, .. } if req.url.ends_with("/posts/create"))
            })
            .count()
    }

    fn last_message_upsert(h: &Harness) -> UpsertSpec {
        h.sink
            .as_slice()
            .iter()
            .rev()
            .find_map(|effect| match effect {
                Effect::Persist { ops, .. }
                | Effect::PersistAtomic { ops, .. }
                | Effect::PersistFire { ops } => ops.iter().find_map(|op| match op {
                    StorageOp::BatchUpsert(spec) if spec.table == "message" => Some(spec.clone()),
                    _ => None,
                }),
                _ => None,
            })
            .expect("message upsert")
    }

    fn row_text<'a>(row: &'a [(String, SqlValue)], key: &str) -> Option<&'a str> {
        row.iter().find_map(|(column, value)| match value {
            SqlValue::Text(text) if column == key => Some(text.as_str()),
            _ => None,
        })
    }

    fn prepare_reply(data: Value) -> Tick {
        port_reply_with_body(
            0,
            enveloped(Bytes::from(
                serde_json::to_vec(&data).expect("prepare reply json"),
            )),
        )
    }

    fn valid_prepare_data() -> Value {
        json!({
            "isSuccess": true,
            "fileId": "file-1",
            "bucket": "attachment",
            "method": "PUT",
            "url": "https://oss.test/media/file-1?signature=short-lived",
            "uri": "media/file-1",
            "objectKey": "media/file-1",
            "headers": {
                "Content-Type": "application/pdf",
                "x-oss-forbid-overwrite": "true"
            }
        })
    }

    #[test]
    fn file_first_step_only_prepares_and_persistence_is_opaque() {
        let mut h = Harness::new(&[1]);
        h.step(file_cmd("tmp-file"), 1000);

        let row = &last_message_upsert(&h).rows[0];
        let persisted = row_text(row, "props").expect("props").to_string();
        release_stage(&mut h, "prepare", 1001);

        let prepares = prepare_effects(&h);
        assert_eq!(prepares.len(), 1);
        assert_eq!(prepares[0].1.method, "POST");
        assert_eq!(
            prepares[0].1.url,
            "http://localhost:3399/oss/getPresignedUrl"
        );
        assert!(upload_effects(&h).is_empty());
        assert_eq!(posts_create_count(&h), 0);
        let prepare_body: Value =
            serde_json::from_slice(prepares[0].1.body.as_ref().expect("prepare body").as_ref())
                .expect("prepare json");
        assert_eq!(prepare_body["mediaUpload"], true);
        assert!(prepare_body["clientUploadId"]
            .as_str()
            .is_some_and(|value| value.ends_with(":file")));
        assert_eq!(prepare_body["bucket"], "attachment");
        assert_eq!(prepare_body["fileName"], "doc.pdf");
        assert_eq!(prepare_body["contentType"], "application/pdf");
        assert_eq!(prepare_body["size"], 9);
        assert!(prepare_body.get("localPath").is_none());

        for secret in [
            "mediaInput",
            "localPath",
            "/tmp/doc.pdf",
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        ] {
            assert!(
                !persisted.contains(secret),
                "persisted props leaked {secret}"
            );
        }
    }

    #[test]
    fn missing_java_base_fails_before_optimistic_sending_is_persisted() {
        let mut module = helix_im::ImModule::new(helix_im::module::ImConfig {
            api_base_url: "http://go.test/api/cses".to_string(),
            default_api_base_url: String::new(),
            ..Default::default()
        });
        let mut out = EffectSink::new();

        let error = module
            .handle(&file_cmd("tmp-no-java"), 1000, &mut out)
            .expect_err("media send without frontend Java base must fail-fast");

        assert!(error.to_string().contains("Java API base is required"));
        assert!(
            out.is_empty(),
            "missing Java endpoint must not leave an optimistic message spinning"
        );
    }

    #[test]
    fn rich_image_uses_the_same_prepare_contract() {
        let mut h = Harness::new(&[1]);
        h.step(rich_cmd(), 1000);
        release_stage(&mut h, "prepare", 1001);

        let prepares = prepare_effects(&h);
        assert_eq!(prepares.len(), 1);
        let body: Value =
            serde_json::from_slice(prepares[0].1.body.as_ref().unwrap().as_ref()).unwrap();
        assert_eq!(body["fileName"], "a.png");
        assert_eq!(body["contentType"], "image/png");
        assert_eq!(body["bucket"], "picture");
        assert!(upload_effects(&h).is_empty());
        assert_eq!(posts_create_count(&h), 0);
    }

    #[test]
    fn mobile_cannot_supply_server_owned_upload_fields() {
        for forbidden in [
            "uploadId",
            "uploadToken",
            "fileId",
            "method",
            "url",
            "publicUrl",
            "objectKey",
            "headers",
            "status",
            "state",
            "userId",
            "teamId",
            "etag",
        ] {
            let mut command = match file_cmd("tmp-forbidden") {
                Tick::Command(command) => {
                    serde_json::from_slice::<Value>(&command.payload).unwrap()
                }
                _ => unreachable!(),
            };
            command["props"]["file"][forbidden] = json!("client-owned");
            let tick = send_message_cmd_json(command);
            let mut h = Harness::new(&[1]);
            let mut out = EffectSink::new();
            let error = h
                .module
                .handle(&tick, 1000, &mut out)
                .expect_err("must reject");
            assert!(
                error.to_string().contains(forbidden),
                "{forbidden}: {error}"
            );
            assert!(out.as_slice().is_empty());
        }
    }

    #[test]
    fn invalid_media_metadata_has_no_http_upload_or_persist_side_effects() {
        let invalid = [
            ("fileName", json!("../doc.pdf")),
            ("fileName", json!("doc\\archive.pdf")),
            ("contentType", json!("application pdf")),
            ("contentType", json!("x".repeat(128))),
            ("size", json!((1_u64 << 30) + 1)),
            (
                "sha256",
                json!("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"),
            ),
            ("sha256", json!("short")),
        ];

        for (field, value) in invalid {
            let mut command = match file_cmd("tmp-invalid-metadata") {
                Tick::Command(command) => {
                    serde_json::from_slice::<Value>(&command.payload).unwrap()
                }
                _ => unreachable!(),
            };
            command["props"]["file"]["mediaInput"][field] = value;
            let tick = send_message_cmd_json(command);
            let mut h = Harness::new(&[1]);
            let mut out = EffectSink::new();
            let error = h
                .module
                .handle(&tick, 1000, &mut out)
                .expect_err("invalid media metadata must fail before effects");

            assert!(error.to_string().contains(field), "{field}: {error}");
            assert!(out.as_slice().is_empty(), "{field} emitted side effects");
        }
    }

    #[test]
    fn prepare_success_builds_exactly_one_upload_from_server_data() {
        let mut h = Harness::new(&[1]);
        h.step(file_cmd("tmp-prepare-ok"), 1000);
        release_stage(&mut h, "prepare", 1001);
        let prepare_corr = prepare_effects(&h)[0].0;
        let outcome = match prepare_reply(valid_prepare_data()) {
            Tick::PortReply { outcome, .. } => outcome,
            _ => unreachable!(),
        };
        h.step(
            Tick::PortReply {
                corr: helix_core::Correlation::from_raw(prepare_corr),
                outcome,
            },
            1010,
        );
        release_stage(&mut h, "put", 1011);

        let uploads = upload_effects(&h);
        assert_eq!(uploads.len(), 1);
        let req = &uploads[0].1;
        assert_eq!(req.local_path, "/tmp/doc.pdf");
        assert_eq!(req.method, "PUT");
        assert_eq!(req.object_key, "media/file-1");
        assert_eq!(
            req.urls.upload_url(),
            "https://oss.test/media/file-1?signature=short-lived"
        );
        assert_eq!(req.urls.public_url(), "media/file-1");
        assert_eq!(req.content_type.as_deref(), Some("application/pdf"));
        assert_eq!(req.size, Some(9));
        assert_eq!(posts_create_count(&h), 0);
    }

    #[test]
    fn failed_or_conflicting_prepare_never_uploads_or_posts() {
        let invalid_data = [
            json!({}),
            json!({
                "isSuccess": true, "fileId": "file-1", "bucket": "attachment",
                "method": "POST", "url": "http://test/upload", "uri": "media/file-1",
                "objectKey": "media/file-1", "headers": {}
            }),
            json!({
                "isSuccess": true, "fileId": "file-1", "bucket": "attachment",
                "method": "PUT", "url": "http://test/upload", "uri": "media/file-1",
                "objectKey": "media/file-1",
                "headers": {
                    "Content-Type": "image/png",
                    "x-oss-forbid-overwrite": "true"
                }
            }),
        ];
        for (index, data) in invalid_data.into_iter().enumerate() {
            let mut h = Harness::new(&[1]);
            h.step(file_cmd(&format!("tmp-invalid-{index}")), 1000);
            release_stage(&mut h, "prepare", 1001);
            let corr = prepare_effects(&h)[0].0;
            let outcome = match prepare_reply(data) {
                Tick::PortReply { outcome, .. } => outcome,
                _ => unreachable!(),
            };
            h.step(
                Tick::PortReply {
                    corr: helix_core::Correlation::from_raw(corr),
                    outcome,
                },
                1010,
            );
            assert!(upload_effects(&h).is_empty());
            assert_eq!(posts_create_count(&h), 0);
        }

        let mut h = Harness::new(&[1]);
        h.step(file_cmd("tmp-prepare-failure"), 1000);
        release_stage(&mut h, "prepare", 1001);
        let corr = prepare_effects(&h)[0].0;
        let response = Bytes::from_static(br#"{"isSuccess":false,"message":"conflict"}"#);
        let outcome = match port_reply_with_body(0, enveloped(response)) {
            Tick::PortReply { outcome, .. } => outcome,
            _ => unreachable!(),
        };
        h.step(
            Tick::PortReply {
                corr: helix_core::Correlation::from_raw(corr),
                outcome,
            },
            1010,
        );
        assert!(upload_effects(&h).is_empty());
        assert_eq!(posts_create_count(&h), 0);
    }

    #[test]
    fn put_success_only_starts_local_completion_without_posts_create() {
        let mut h = Harness::new(&[1]);
        h.step(file_cmd("tmp-put-ok"), 1000);
        release_stage(&mut h, "prepare", 1001);
        let prepare_corr = prepare_effects(&h)[0].0;
        let outcome = match prepare_reply(valid_prepare_data()) {
            Tick::PortReply { outcome, .. } => outcome,
            _ => unreachable!(),
        };
        h.step(
            Tick::PortReply {
                corr: helix_core::Correlation::from_raw(prepare_corr),
                outcome,
            },
            1010,
        );
        release_stage(&mut h, "put", 1011);
        let upload_corr = upload_effects(&h)[0].0;
        h.step(port_reply_ok(upload_corr), 1020);
        let complete_persist = pending_stage_corr(&h, "complete");
        let effects = h.step(port_reply_ok(complete_persist), 1021);

        assert_eq!(
            effects
                .iter()
                .filter(|effect| matches!(effect, EffectKind::Http(_)))
                .count(),
            0,
            "PUT success must not call Java complete"
        );
        assert!(effects
            .iter()
            .any(|effect| matches!(effect, EffectKind::PersistAtomic(_))));
        assert_eq!(posts_create_count(&h), 0);
    }
}
