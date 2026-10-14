mod common;

use bytes::Bytes;
use common::{
    cid, command_json, enveloped, port_reply_err, port_reply_ok, port_reply_with_body, timer_tick,
    Harness,
};
use helix_core::effect::{Effect, SqlValue, StorageOp};
use helix_core::Tick;
use serde_json::{json, Value};

const SIZE: u64 = 100 * 1024 * 1024 + 1;
const PART_SIZE: u64 = 8 * 1024 * 1024;

fn schedule_cmd(req_id: &str) -> Tick {
    command_json(
        "im_create_schedule",
        json!({
            "channel_id": cid(1),
            "message": "",
            "type": "FILE",
            "req_id": req_id,
            "schedule_post_at": 1_900_000_000_000_i64,
            "props": {"file": {"mediaInput": {
                "localPath": "/tmp/schedule-multipart.bin",
                "fileName": "schedule-multipart.bin",
                "contentType": "application/octet-stream",
                "size": SIZE,
                "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
            }}}
        }),
    )
}

fn rich_schedule_cmd(req_id: &str) -> Tick {
    command_json(
        "im_create_schedule",
        json!({
            "channel_id": cid(1),
            "message": "",
            "type": "RICH",
            "req_id": req_id,
            "schedule_post_at": 1_900_000_000_000_i64,
            "props": {
                "imagePosition": "top",
                "files": [
                    {"mediaInput": {
                        "localPath": "/tmp/schedule-large.png",
                        "fileName": "schedule-large.png",
                        "contentType": "video/mp4",
                        "size": SIZE,
                        "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                    }},
                    {"mediaInput": {
                        "localPath": "/tmp/schedule-small.png",
                        "fileName": "schedule-small.png",
                        "contentType": "video/mp4",
                        "size": 9,
                        "sha256": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
                    }}
                ]
            }
        }),
    )
}

fn java_reply(corr: u64, body: Value) -> Tick {
    port_reply_with_body(
        corr,
        enveloped(Bytes::from(
            serde_json::to_vec(&body).expect("fixture JSON"),
        )),
    )
}

fn http_corr(h: &Harness, suffix: &str) -> u64 {
    h.sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Http { corr, req } if req.url.ends_with(suffix) => Some(corr.raw()),
            _ => None,
        })
        .expect("expected HTTP effect")
}

fn http_body(h: &Harness, suffix: &str) -> Value {
    h.sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Http { req, .. } if req.url.ends_with(suffix) => req
                .body
                .as_ref()
                .and_then(|body| serde_json::from_slice(body.as_ref()).ok()),
            _ => None,
        })
        .expect("expected HTTP JSON body")
}

fn persist_corr(h: &Harness, table: &str, stage: Option<&str>) -> u64 {
    h.sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::PersistAtomic { corr, ops }
                if ops.iter().any(|op| match op {
                    StorageOp::BatchUpsert(spec) if spec.table == table => {
                        stage.is_none_or(|expected| {
                            spec.rows.iter().flatten().any(|(name, value)| {
                                name == "stage"
                                    && matches!(value, SqlValue::Text(actual) if actual == expected)
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
        .expect("expected matching PersistAtomic")
}

fn init_reply() -> Value {
    json!({
        "isSuccess": true,
        "protocolVersion": 1,
        "status": "uploading",
        "uploadId": "schedule-up-1",
        "fileId": "schedule-file-1",
        "bucket": "attachment",
        "objectKey": "media/schedule-file-1",
        "uri": "media/schedule-file-1",
        "partSize": PART_SIZE,
        "size": SIZE,
        "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
    })
}

fn parts_reply(parts: Value, status: &str) -> Value {
    json!({"isSuccess": true, "uploadId": "schedule-up-1", "status": status, "parts": parts})
}

fn signed_reply(count: u32) -> Value {
    json!({
        "isSuccess": true,
        "uploadId": "schedule-up-1",
        "expiresAt": 1_900_000_300_000_u64,
        "parts": (1..=count).map(|part_number| json!({
            "partNumber": part_number,
            "url": format!("https://oss.test/schedule/{part_number}?sig=secret"),
            "headers": {"Content-Type": "application/octet-stream"}
        })).collect::<Vec<_>>()
    })
}

fn complete_parts() -> Value {
    let count = SIZE.div_ceil(PART_SIZE);
    json!((1..=count)
        .map(|part_number| json!({
            "partNumber": part_number,
            "size": (SIZE - (part_number - 1) * PART_SIZE).min(PART_SIZE),
            "etag": format!("schedule-etag-{part_number}")
        }))
        .collect::<Vec<_>>())
}

fn complete_reply() -> Value {
    json!({
        "isSuccess": true,
        "status": "completed",
        "uploadId": "schedule-up-1",
        "fileId": "schedule-file-1",
        "bucket": "attachment",
        "objectKey": "media/schedule-file-1",
        "uri": "media/schedule-file-1",
        "size": SIZE,
        "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
    })
}

fn schedule_scan_corr(h: &Harness) -> u64 {
    h.sink
        .as_slice()
        .iter()
        .find_map(|effect| {
            match effect {
            Effect::Persist { corr, ops }
                if ops.iter().any(|op| {
                    matches!(op, StorageOp::Scan(spec) if spec.table == "schedule_media_intent")
                }) => Some(corr.raw()),
            _ => None,
        }
        })
        .expect("on_start must scan durable schedule intents")
}

fn schedule_media_row(
    channel: u64,
    account_id: &str,
    owner_user_id: &str,
    request_id: &str,
    stage: &str,
    media_json: Value,
) -> Value {
    let payload = json!({
        "channel_id": cid(channel),
        "message": "",
        "type": "FILE",
        "req_id": request_id,
        "schedule_post_at": 1_900_000_000_000_i64,
        "props": {"file": {"upload": {"status": "preparing"}}}
    });
    json!({
        "channel_id": cid(channel),
        "request_id": request_id,
        "account_id": account_id,
        "owner_user_id": owner_user_id,
        "schedule_post_at": 1_900_000_000_000_i64,
        "payload_json": serde_json::to_string(&payload).expect("payload fixture"),
        "media_json": serde_json::to_string(&media_json).expect("media fixture"),
        "stage": stage,
        "session_id": "",
        "stable_object_ref": "",
        "last_error": ""
    })
}

fn schedule_media_value() -> Value {
    json!([{
        "target": {"kind": "file"},
        "localPath": "/tmp/schedule-recovered.bin",
        "fileName": "schedule-recovered.bin",
        "contentType": "application/octet-stream",
        "size": SIZE,
        "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
    }])
}

fn timer_after(h: &Harness, after_ms: u64) -> Option<u64> {
    h.sink.as_slice().iter().find_map(|effect| match effect {
        Effect::ScheduleTimer {
            id,
            after_ms: actual,
        } if *actual == after_ms => Some(id.raw()),
        _ => None,
    })
}

fn schedule_to_first_upload_batch(h: &mut Harness, req_id: &str, base: u64) -> Vec<u64> {
    h.step(schedule_cmd(req_id), base);
    let intent = persist_corr(h, "schedule_media_intent", Some("intent"));
    h.step(port_reply_ok(intent), base + 1);
    let init = http_corr(h, "/oss/multipart/init");
    h.step(java_reply(init, init_reply()), base + 2);
    let init_stage = persist_corr(h, "schedule_media_intent", Some("parts"));
    h.step(port_reply_ok(init_stage), base + 3);
    let parts = http_corr(h, "/oss/multipart/parts");
    h.step(
        java_reply(parts, parts_reply(json!([]), "uploading")),
        base + 4,
    );
    let sign_stage = persist_corr(h, "schedule_media_intent", Some("sign"));
    h.step(port_reply_ok(sign_stage), base + 5);
    let sign = http_corr(h, "/oss/multipart/sign");
    h.step(java_reply(sign, signed_reply(8)), base + 6);
    h.sink
        .as_slice()
        .iter()
        .filter_map(|effect| match effect {
            Effect::UploadFile { corr, .. } => Some(corr.raw()),
            _ => None,
        })
        .collect()
}

#[test]
fn schedule_multipart_reaches_create_only_after_part_checkpoints_and_stable_persist() {
    let mut h = Harness::new(&[1]);
    h.step(
        schedule_cmd("schedule-multipart-lifecycle"),
        1_700_000_000_000,
    );

    let intent = persist_corr(&h, "schedule_media_intent", Some("intent"));
    h.step(port_reply_ok(intent), 1_700_000_000_001);
    let init = http_corr(&h, "/oss/multipart/init");
    h.step(java_reply(init, init_reply()), 1_700_000_000_002);
    assert_eq!(timer_after(&h, 300_000).is_some(), true);

    let init_stage = persist_corr(&h, "schedule_media_intent", Some("parts"));
    assert!(h.sink.as_slice().iter().any(|effect| matches!(
        effect,
        Effect::PersistAtomic { ops, .. }
            if ops.iter().any(|op| matches!(op, StorageOp::BatchUpsert(spec) if spec.table == "media_upload_session"))
    )));
    h.step(port_reply_ok(init_stage), 1_700_000_000_003);
    let parts = http_corr(&h, "/oss/multipart/parts");
    h.step(
        java_reply(parts, parts_reply(json!([]), "uploading")),
        1_700_000_000_004,
    );

    let sign_stage = persist_corr(&h, "schedule_media_intent", Some("sign"));
    h.step(port_reply_ok(sign_stage), 1_700_000_000_005);
    let sign = http_corr(&h, "/oss/multipart/sign");
    h.step(java_reply(sign, signed_reply(8)), 1_700_000_000_006);

    let uploads = h
        .sink
        .as_slice()
        .iter()
        .filter_map(|effect| match effect {
            Effect::UploadFile { corr, req } => Some((corr.raw(), req.clone())),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        uploads.len(),
        3,
        "schedule file has at most three in-flight PUTs"
    );
    assert_eq!(uploads[0].1.size, Some(SIZE));
    assert_eq!(
        uploads[0]
            .1
            .range()
            .map(|range| (range.offset, range.length)),
        Some((0, PART_SIZE))
    );

    let mut pending_uploads = uploads;
    let mut index = 0_u32;
    while let Some((corr, _)) = pending_uploads.pop() {
        h.step(
            port_reply_with_body(
                corr,
                Bytes::from(
                    serde_json::to_vec(&json!({
                        "objectKey": "media/schedule-file-1",
                        "url": "media/schedule-file-1",
                        "etag": format!("schedule-etag-{}", index + 1)
                    }))
                    .expect("UploadFile fixture"),
                ),
            ),
            1_700_000_000_007 + u64::from(index) * 2,
        );
        let checkpoint = persist_corr(&h, "media_upload_part", None);
        assert!(h.sink.as_slice().iter().all(|effect| match effect {
            Effect::PersistAtomic { ops, .. } => ops.iter().all(|op| {
                !matches!(op, StorageOp::BatchUpsert(spec) if spec.table == "schedule_media_intent")
                    && !matches!(op, StorageOp::BatchUpsert(spec) if spec.table == "media_upload_session")
            }),
            _ => true,
        }));
        h.step(
            port_reply_ok(checkpoint),
            1_700_000_000_008 + u64::from(index) * 2,
        );
        index += 1;
        pending_uploads.extend(h.sink.as_slice().iter().filter_map(|effect| match effect {
            Effect::UploadFile { corr, req } => Some((corr.raw(), req.clone())),
            _ => None,
        }));
    }

    let requery = http_corr(&h, "/oss/multipart/parts");
    h.step(
        java_reply(requery, parts_reply(complete_parts(), "completed")),
        1_700_000_000_015,
    );

    let complete_stage = persist_corr(&h, "schedule_media_intent", Some("parts"));
    h.step(port_reply_ok(complete_stage), 1_700_000_000_016);
    let complete = http_corr(&h, "/oss/multipart/complete");
    h.step(java_reply(complete, complete_reply()), 1_700_000_000_017);

    let stable = h
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::PersistAtomic { corr, ops }
                if ops.iter().any(|op| matches!(
                    op,
                    StorageOp::BatchUpsert(spec)
                        if spec.table == "schedule_media_intent"
                            && spec.rows.iter().flatten().any(|(name, value)| {
                                name == "stage"
                                    && matches!(value, SqlValue::Text(actual) if actual == "submitting")
                            })
                )) => Some(corr.raw()),
            _ => None,
        })
        .expect("stable reference must be behind submitting PersistAtomic");
    assert!(!h.sink.as_slice().iter().any(|effect| matches!(
        effect,
        Effect::Http { req, .. } if req.url.ends_with("/posts/createSchedule")
    )));
    h.step(port_reply_ok(stable), 1_700_000_000_018);
    assert!(h.sink.as_slice().iter().any(|effect| matches!(
        effect,
        Effect::Http { req, .. } if req.url.ends_with("/posts/createSchedule")
    )));
    assert!(!h.sink.as_slice().iter().any(|effect| matches!(
        effect,
        Effect::Http { req, .. } if req.url.ends_with("/posts/create")
    )));
    let create_schedule = http_corr(&h, "/posts/createSchedule");
    h.step(port_reply_err(create_schedule), 1_700_000_000_019);
    assert!(h.sink.as_slice().iter().any(|effect| {
        matches!(
            effect,
            Effect::PersistAtomic { ops, .. }
                if ops.iter().any(|op| matches!(
                    op,
                    StorageOp::BatchUpdate(spec)
                        if spec.table == "schedule_media_intent"
                ))
        )
    }));
    assert!(h
        .sink
        .as_slice()
        .iter()
        .any(|effect| matches!(effect, Effect::Emit { .. })));

    h.step(schedule_cmd("schedule-unknown-retry"), 1_700_000_000_020);
    assert!(h.sink.as_slice().iter().all(|effect| {
        !matches!(
            effect,
            Effect::PersistAtomic { ops, .. }
                if ops.iter().any(|op| matches!(
                    op,
                    StorageOp::BatchUpsert(spec) if spec.table == "schedule_media_intent"
                ))
        )
    }));

    h.step(
        command_json(
            "im_cancel_schedule",
            json!({"channel_id": cid(1), "req_id": "schedule-unknown-cancel"}),
        ),
        1_700_000_000_021,
    );
    h.step(
        Tick::Inbound(helix_core::tick::InboundBytes(Bytes::from(
            serde_json::to_vec(&json!({
                "action": "post_schedule_canceled",
                "data": {
                    "channelId": cid(1),
                    "scheduleOwnerUserId": "actor",
                    "status": "canceled",
                    "revision": 8
                }
            }))
            .expect("unknown cancellation authority fixture"),
        ))),
        1_700_000_000_022,
    );
    let cancel_persist = h
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::PersistAtomic { corr, ops }
                if ops.iter().any(|op| {
                    matches!(op, StorageOp::BatchDelete(spec) if spec.table == "schedule_media_intent")
                }) =>
            {
                Some(corr.raw())
            }
            _ => None,
        })
        .expect("unknown submission cancel must clear durable intent atomically");
    h.step(port_reply_ok(cancel_persist), 1_700_000_000_023);
    h.step(
        schedule_cmd("schedule-after-unknown-cancel"),
        1_700_000_000_024,
    );
    assert!(h.sink.as_slice().iter().any(|effect| {
        matches!(effect, Effect::PersistAtomic { ops, .. } if ops.iter().any(|op| matches!(op, StorageOp::BatchUpsert(spec) if spec.table == "schedule_media_intent")))
    }));
}

#[test]
fn schedule_multipart_attachment_switch_deletes_old_journal_before_next_upload() {
    let mut h = Harness::new(&[1]);
    h.step(rich_schedule_cmd("schedule-mixed-media"), 1_700_000_000_000);
    let intent = persist_corr(&h, "schedule_media_intent", Some("intent"));
    h.step(port_reply_ok(intent), 1_700_000_000_001);
    let init = http_corr(&h, "/oss/multipart/init");
    h.step(java_reply(init, init_reply()), 1_700_000_000_002);
    let init_stage = persist_corr(&h, "schedule_media_intent", Some("parts"));
    h.step(port_reply_ok(init_stage), 1_700_000_000_003);
    let parts = http_corr(&h, "/oss/multipart/parts");
    h.step(
        java_reply(parts, parts_reply(complete_parts(), "completed")),
        1_700_000_000_004,
    );
    let complete_stage = persist_corr(&h, "schedule_media_intent", Some("parts"));
    h.step(port_reply_ok(complete_stage), 1_700_000_000_005);
    let complete = http_corr(&h, "/oss/multipart/complete");
    h.step(java_reply(complete, complete_reply()), 1_700_000_000_006);

    let switch_persist = h
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::PersistAtomic { corr, ops }
                if ops.iter().any(|op| {
                    matches!(op, StorageOp::BatchDelete(spec) if spec.table == "media_upload_session")
                }) =>
            {
                Some((corr.raw(), ops.clone()))
            }
            _ => None,
        })
        .expect("switch checkpoint must delete completed attachment journal");
    assert!(switch_persist.1.iter().any(|op| {
        matches!(op, StorageOp::BatchDelete(spec) if spec.table == "media_upload_part")
    }));
    assert!(h.sink.as_slice().iter().all(|effect| {
        !matches!(effect, Effect::Http { req, .. } if req.url.ends_with("/oss/getPresignedUrl"))
    }));

    h.step(port_reply_ok(switch_persist.0), 1_700_000_000_007);
    assert!(h.sink.as_slice().iter().any(|effect| {
        matches!(effect, Effect::Http { req, .. } if req.url.ends_with("/oss/getPresignedUrl"))
    }));
}

#[test]
fn schedule_rehydrate_requeries_remote_parts_instead_of_trusting_sparse_rows() {
    let mut h = Harness::new_with_identity(&[1], "user-1", "team-1", "", "", "");
    h.start(1_700_000_000_000);
    let scan = schedule_scan_corr(&h);
    let row = schedule_media_row(
        1,
        "team-1",
        "user-1",
        "schedule-recover",
        "parts",
        schedule_media_value(),
    );
    h.step(
        port_reply_with_body(
            scan,
            Bytes::from(serde_json::to_vec(&json!([row])).unwrap()),
        ),
        1_700_000_000_001,
    );
    let init = http_corr(&h, "/oss/multipart/init");
    h.step(java_reply(init, init_reply()), 1_700_000_000_002);
    let stage = persist_corr(&h, "schedule_media_intent", Some("parts"));
    h.step(port_reply_ok(stage), 1_700_000_000_003);
    let parts = http_corr(&h, "/oss/multipart/parts");
    assert!(h
        .sink
        .as_slice()
        .iter()
        .all(|effect| { !matches!(effect, Effect::UploadFile { .. }) }));
    h.step(
        java_reply(parts, parts_reply(json!([]), "uploading")),
        1_700_000_000_004,
    );
    let sign_stage = persist_corr(&h, "schedule_media_intent", Some("sign"));
    h.step(port_reply_ok(sign_stage), 1_700_000_000_005);
    assert!(h
        .sink
        .as_slice()
        .iter()
        .any(|effect| matches!(effect, Effect::Http { req, .. } if req.url.ends_with("/oss/multipart/sign"))));
}

#[test]
fn schedule_rehydrate_foreign_owner_is_read_only() {
    let mut h = Harness::new_with_identity(&[1], "user-1", "team-1", "", "", "");
    h.start(1_700_000_000_000);
    let scan = schedule_scan_corr(&h);
    let row = schedule_media_row(
        1,
        "other-team",
        "other-user",
        "foreign-request",
        "parts",
        schedule_media_value(),
    );
    h.step(
        port_reply_with_body(
            scan,
            Bytes::from(serde_json::to_vec(&json!([row])).unwrap()),
        ),
        1_700_000_000_001,
    );
    assert!(h.sink.as_slice().iter().all(|effect| {
        !matches!(effect, Effect::Http { .. } | Effect::UploadFile { .. })
            && !matches!(effect, Effect::PersistAtomic { ops, .. } if ops.iter().any(|op| matches!(op, StorageOp::BatchUpdate(spec) if spec.table == "schedule_media_intent")))
            && !matches!(effect, Effect::Emit { event } if event.0.as_ref().windows(b"foreign-request".len()).any(|window| window == b"foreign-request"))
    }));
}

#[test]
fn schedule_cancel_authority_does_not_delete_foreign_recovery_intent() {
    let mut h = Harness::new_with_identity(&[1], "user-1", "team-1", "", "", "");
    h.start(1_700_000_000_000);
    let scan = schedule_scan_corr(&h);
    let row = schedule_media_row(
        1,
        "other-team",
        "other-user",
        "foreign-cancel",
        "parts",
        schedule_media_value(),
    );
    h.step(
        port_reply_with_body(
            scan,
            Bytes::from(serde_json::to_vec(&json!([row])).unwrap()),
        ),
        1_700_000_000_001,
    );

    // The current owner may cancel its channel, but that authority must not
    // delete a recovery row owned by another account.
    h.step(
        command_json(
            "im_cancel_schedule",
            json!({"channel_id": cid(1), "req_id": "cancel-foreign"}),
        ),
        1_700_000_000_002,
    );
    h.step(
        Tick::Inbound(helix_core::tick::InboundBytes(Bytes::from(
            serde_json::to_vec(&json!({
                "action": "post_schedule_canceled",
                "data": {
                    "channelId": cid(1),
                    "scheduleOwnerUserId": "user-1",
                    "status": "canceled",
                    "revision": 7
                }
            }))
            .expect("cancel authority fixture"),
        ))),
        1_700_000_000_003,
    );

    let ops = h
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::PersistAtomic { ops, .. }
                if ops.iter().any(
                    |op| matches!(op, StorageOp::BatchUpdate(spec) if spec.table == "channel"),
                ) =>
            {
                Some(ops)
            }
            _ => None,
        })
        .expect("cancel authority must still persist the channel fact");
    assert_eq!(ops.len(), 2);
    assert!(ops.iter().all(|op| {
        !matches!(
            op,
            StorageOp::BatchDelete(spec) if spec.table == "schedule_media_intent"
        )
    }));
}

#[test]
fn schedule_rehydrate_bad_row_does_not_block_later_valid_row() {
    let mut h = Harness::new_with_identity(&[1, 2], "user-1", "team-1", "", "", "");
    h.start(1_700_000_000_000);
    let scan = schedule_scan_corr(&h);
    let bad = schedule_media_row(
        2,
        "team-1",
        "user-1",
        "bad-request",
        "parts",
        json!([{"target": {"kind": "file"}}]),
    );
    let valid = schedule_media_row(
        1,
        "team-1",
        "user-1",
        "valid-request",
        "parts",
        schedule_media_value(),
    );
    h.step(
        port_reply_with_body(
            scan,
            Bytes::from(serde_json::to_vec(&json!([bad, valid])).unwrap()),
        ),
        1_700_000_000_001,
    );
    assert!(h
        .sink
        .as_slice()
        .iter()
        .any(|effect| matches!(effect, Effect::Http { req, .. } if req.url.ends_with("/oss/multipart/init"))));
    assert!(h.sink.as_slice().iter().any(|effect| {
        matches!(effect, Effect::PersistAtomic { ops, .. } if ops.iter().any(|op| matches!(op, StorageOp::BatchUpdate(spec) if spec.table == "schedule_media_intent")))
    }));
}

#[test]
fn schedule_rehydrate_keeps_excess_valid_rows_durable_at_capacity() {
    let mut h = Harness::new_with_identity(&[1], "user-1", "team-1", "", "", "");
    h.start(1_700_000_000_000);
    let scan = schedule_scan_corr(&h);
    let rows = (1..=33_u64)
        .map(|channel| {
            schedule_media_row(
                channel,
                "team-1",
                "user-1",
                &format!("schedule-capacity-{channel}"),
                "parts",
                schedule_media_value(),
            )
        })
        .collect::<Vec<_>>();
    h.step(
        port_reply_with_body(
            scan,
            Bytes::from(serde_json::to_vec(&rows).expect("capacity rows")),
        ),
        1_700_000_000_001,
    );
    let init_count = h
        .sink
        .as_slice()
        .iter()
        .filter(|effect| {
            matches!(effect, Effect::Http { req, .. } if req.url.ends_with("/oss/multipart/init"))
        })
        .count();
    assert_eq!(
        init_count, 32,
        "startup recovery must keep the 32-intent cap"
    );
}

#[test]
fn schedule_rehydrate_expired_owner_marks_failed_without_network_io() {
    let mut h = Harness::new_with_identity(&[1], "user-1", "team-1", "", "", "");
    h.start(1_900_000_000_001);
    let scan = schedule_scan_corr(&h);
    let row = schedule_media_row(
        1,
        "team-1",
        "user-1",
        "expired-request",
        "parts",
        schedule_media_value(),
    );
    h.step(
        port_reply_with_body(
            scan,
            Bytes::from(serde_json::to_vec(&json!([row])).unwrap()),
        ),
        1_900_000_000_001,
    );
    assert!(h
        .sink
        .as_slice()
        .iter()
        .all(|effect| { !matches!(effect, Effect::Http { .. } | Effect::UploadFile { .. }) }));
    assert!(h.sink.as_slice().iter().any(|effect| {
        matches!(effect, Effect::PersistAtomic { ops, .. } if ops.iter().any(|op| matches!(op, StorageOp::BatchUpdate(spec) if spec.table == "schedule_media_intent" && spec.patch.iter().any(|(name, value)| name == "stage" && matches!(value, SqlValue::Text(stage) if stage == "failed")))))
    }));
}

#[test]
fn schedule_multipart_persist_error_releases_context_and_ignores_late_reply() {
    let mut h = Harness::new(&[1]);
    h.step(schedule_cmd("schedule-persist-error"), 1_700_000_000_000);
    let intent = persist_corr(&h, "schedule_media_intent", Some("intent"));
    h.step(port_reply_err(intent), 1_700_000_000_001);
    assert!(h
        .sink
        .as_slice()
        .iter()
        .all(|effect| { !matches!(effect, Effect::Http { .. } | Effect::UploadFile { .. }) }));
    let failed_stage = persist_corr(&h, "schedule_media_intent", Some("failed"));
    h.step(port_reply_ok(failed_stage), 1_700_000_000_002);
    assert!(h
        .sink
        .as_slice()
        .iter()
        .any(|effect| matches!(effect, Effect::Emit { .. })));
    h.step(port_reply_ok(intent), 1_700_000_000_002);
    assert!(h
        .sink
        .as_slice()
        .iter()
        .all(|effect| { !matches!(effect, Effect::Http { .. } | Effect::UploadFile { .. }) }));
}

#[test]
fn schedule_multipart_timeout_drains_old_init_before_bounded_retry() {
    let mut h = Harness::new(&[1]);
    h.step(schedule_cmd("schedule-timeout"), 1_700_000_000_000);
    let intent = persist_corr(&h, "schedule_media_intent", Some("intent"));
    h.step(port_reply_ok(intent), 1_700_000_000_001);
    let init = http_corr(&h, "/oss/multipart/init");
    let initial_identity = http_body(&h, "/oss/multipart/init")["clientUploadId"].clone();
    let watchdog = timer_after(&h, 300_000).expect("multipart watchdog");
    h.step(timer_tick(watchdog), 1_700_000_300_002);
    assert!(
        timer_after(&h, 100).is_none(),
        "retry waits for old init route"
    );
    h.step(port_reply_err(init), 1_700_000_300_003);
    let retry_timer = timer_after(&h, 100).expect("first bounded retry timer");
    h.step(timer_tick(retry_timer), 1_700_000_300_104);
    let retry_init = http_corr(&h, "/oss/multipart/init");
    assert_eq!(
        http_body(&h, "/oss/multipart/init")["clientUploadId"],
        initial_identity,
        "automatic retry must retain the durable clientUploadId"
    );
    h.step(port_reply_err(retry_init), 1_700_000_300_105);
    let second_timer = timer_after(&h, 200).expect("second bounded retry timer");
    h.step(timer_tick(second_timer), 1_700_000_300_306);
    let second_init = http_corr(&h, "/oss/multipart/init");
    h.step(port_reply_err(second_init), 1_700_000_300_307);
    let third_timer = timer_after(&h, 400).expect("third bounded retry timer");
    h.step(timer_tick(third_timer), 1_700_000_300_708);
    let third_init = http_corr(&h, "/oss/multipart/init");
    h.step(port_reply_err(third_init), 1_700_000_300_709);
    assert!(timer_after(&h, 100).is_none());
    let failed_stage = persist_corr(&h, "schedule_media_intent", Some("failed"));
    h.step(port_reply_ok(failed_stage), 1_700_000_300_710);
    assert!(h
        .sink
        .as_slice()
        .iter()
        .any(|effect| matches!(effect, Effect::Emit { .. })));
}

#[test]
fn schedule_multipart_progress_replaces_watchdog_and_ignores_stale_timer() {
    let mut h = Harness::new(&[1]);
    h.step(schedule_cmd("schedule-watchdog"), 1_700_000_000_000);
    let intent = persist_corr(&h, "schedule_media_intent", Some("intent"));
    h.step(port_reply_ok(intent), 1_700_000_000_001);
    let old_timer = timer_after(&h, 300_000).expect("initial watchdog");
    let init = http_corr(&h, "/oss/multipart/init");
    h.step(java_reply(init, init_reply()), 1_700_000_000_002);
    let new_timer = timer_after(&h, 300_000).expect("rearmed watchdog");
    assert_ne!(old_timer, new_timer, "progress must rotate timer identity");

    h.step(timer_tick(old_timer), 1_700_000_300_003);
    assert!(h.sink.as_slice().is_empty(), "stale timer must be ignored");

    h.step(timer_tick(new_timer), 1_700_000_300_004);
    assert!(h
        .sink
        .as_slice()
        .iter()
        .any(|effect| matches!(effect, Effect::CancelTimer { .. })));
}

#[test]
fn schedule_multipart_drain_waits_for_sibling_put_and_checkpoint_replies() {
    let mut h = Harness::new(&[1]);
    let uploads = schedule_to_first_upload_batch(&mut h, "schedule-drain", 1_700_000_000_000);
    assert_eq!(uploads.len(), 3);

    // The first confirmed PUT has an outstanding single-row checkpoint. A
    // sibling failure must wait for both the third PUT and that checkpoint.
    h.step(
        port_reply_with_body(
            uploads[0],
            Bytes::from(
                serde_json::to_vec(&json!({
                    "objectKey": "media/schedule-file-1",
                    "url": "media/schedule-file-1",
                    "etag": "schedule-etag-1"
                }))
                .unwrap(),
            ),
        ),
        1_700_000_000_007,
    );
    let checkpoint = persist_corr(&h, "media_upload_part", None);
    h.step(port_reply_err(uploads[1]), 1_700_000_000_008);
    assert!(timer_after(&h, 100).is_none());
    h.step(port_reply_ok(uploads[2]), 1_700_000_000_009);
    assert!(timer_after(&h, 100).is_none());
    h.step(port_reply_ok(checkpoint), 1_700_000_000_010);
    let retry = timer_after(&h, 100).expect("drain must finish before retry timer");
    h.step(timer_tick(retry), 1_700_000_000_111);
    assert!(h
        .sink
        .as_slice()
        .iter()
        .any(|effect| matches!(effect, Effect::Http { req, .. } if req.url.ends_with("/oss/multipart/init"))));
}

#[test]
fn schedule_multipart_oversize_fails_before_durable_or_network_effects() {
    let mut h = Harness::new(&[1]);
    h.step(
        command_json(
            "im_create_schedule",
            json!({
                "channel_id": cid(1), "message": "", "type": "FILE", "req_id": "schedule-oversize",
                "schedule_post_at": 1_900_000_000_000_i64,
                "props": {"file": {"mediaInput": {
                    "localPath": "/tmp/oversize.bin", "fileName": "oversize.bin",
                    "contentType": "application/octet-stream", "size": (1_u64 << 30) + 1,
                    "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                }}}
            }),
        ),
        1_700_000_000_000,
    );
    assert!(h.sink.as_slice().iter().all(|effect| {
        !matches!(effect, Effect::Http { .. } | Effect::UploadFile { .. })
            && !matches!(effect, Effect::PersistAtomic { ops, .. } if ops.iter().any(|op| matches!(op, StorageOp::BatchUpsert(spec) if spec.table == "schedule_media_intent")))
    }));
}

#[test]
fn schedule_cancel_authority_deletes_intent_and_releases_channel_after_persist() {
    let mut h = Harness::new(&[1]);
    h.step(schedule_cmd("schedule-cancel-fence"), 1_700_000_000_000);
    let intent = persist_corr(&h, "schedule_media_intent", Some("intent"));
    h.step(port_reply_ok(intent), 1_700_000_000_001);
    let init = http_corr(&h, "/oss/multipart/init");

    h.step(
        command_json(
            "im_cancel_schedule",
            json!({"channel_id": cid(1), "req_id": "schedule-cancel"}),
        ),
        1_700_000_000_002,
    );
    h.step(
        Tick::Inbound(helix_core::tick::InboundBytes(Bytes::from(
            serde_json::to_vec(&json!({
                "action": "post_schedule_canceled",
                "data": {
                    "channelId": cid(1),
                    "scheduleOwnerUserId": "actor",
                    "status": "canceled",
                    "revision": 7
                }
            }))
            .expect("cancel authority fixture"),
        ))),
        1_700_000_000_003,
    );
    let cancel_persist = h
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::PersistAtomic { corr, ops }
                if ops.iter().any(|op| {
                    matches!(op, StorageOp::BatchDelete(spec) if spec.table == "schedule_media_intent")
                }) =>
            {
                Some(corr.raw())
            }
            _ => None,
        })
        .expect("cancel authority must delete durable multipart intent atomically");
    h.step(port_reply_ok(cancel_persist), 1_700_000_000_004);

    // The old PUT/init reply can arrive after cancellation, but it must not
    // resurrect the independent multipart state or schedule another request.
    h.step(java_reply(init, init_reply()), 1_700_000_000_005);
    assert!(h.sink.as_slice().iter().all(|effect| {
        !matches!(effect, Effect::Http { req, .. } if req.url.ends_with("/oss/multipart/parts"))
            && !matches!(effect, Effect::UploadFile { .. })
    }));

    h.step(schedule_cmd("schedule-after-cancel"), 1_700_000_000_006);
    assert!(h.sink.as_slice().iter().any(|effect| {
        matches!(effect, Effect::PersistAtomic { ops, .. } if ops.iter().any(|op| matches!(op, StorageOp::BatchUpsert(spec) if spec.table == "schedule_media_intent")))
    }));
}
