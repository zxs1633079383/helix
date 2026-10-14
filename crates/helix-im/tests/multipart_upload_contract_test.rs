mod common;

use bytes::Bytes;
use common::{
    cid, enveloped, port_reply_err, port_reply_ok, port_reply_with_body, send_message_cmd_json,
    timer_tick, Harness,
};
use helix_core::effect::Effect;
use helix_core::effect::{FileUploadProgress, FileUploadRequest, SqlValue, StorageOp};
use helix_core::tick::{PortError, PortOutcome};
use helix_core::{Correlation, Tick};
use helix_im::send::upload_props::{FailedMediaOp, MULTIPART_THRESHOLD_BYTES};
use serde_json::json;

const PART_SIZE: u64 = 8 * 1024 * 1024;
const MULTIPART_TEST_SIZE: u64 = MULTIPART_THRESHOLD_BYTES + 1;

fn large_file_send() -> helix_core::Tick {
    send_message_cmd_json(json!({
        "channel_id": cid(1),
        "temporary_id": "tmp-multipart-red",
        "type": "FILE",
        "text": "",
        "props": {
            "file": {
                "mediaInput": {
                    "localPath": "/tmp/multipart-red.bin",
                    "fileName": "multipart-red.bin",
                    "contentType": "application/octet-stream",
                    "size": MULTIPART_TEST_SIZE,
                    "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                }
            }
        }
    }))
}

/// 公开 Command seam：超过 100 MiB 的文件必须从 Java multipart init 开始，不能落回旧 PUT。
#[test]
fn large_file_enters_multipart_init_after_optimistic_persist() {
    let mut harness = Harness::new(&[1]);
    let _ = harness.step(large_file_send(), 1_700_000_000_000);

    assert!(harness
        .sink
        .as_slice()
        .iter()
        .any(|effect| matches!(effect, Effect::PersistAtomic { .. })));
    let persist_corr = harness
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::PersistAtomic { corr, .. } => Some(corr.raw()),
            _ => None,
        })
        .expect("multipart send must expose optimistic persist correlation");
    harness.step(port_reply_ok(persist_corr), 1_700_000_000_001);
    assert!(harness.sink.as_slice().iter().any(|effect| matches!(
        effect,
        Effect::Http { req, .. } if req.url.ends_with("/oss/multipart/init")
    )));
}

#[test]
fn threshold_file_keeps_legacy_prepare_path() {
    let mut harness = Harness::new(&[1]);
    let _ = harness.step(
        large_file_send_size("tmp-multipart-threshold", MULTIPART_THRESHOLD_BYTES),
        1_700_000_000_000,
    );
    let persist_corr = harness
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::PersistAtomic { corr, .. } => Some(corr.raw()),
            _ => None,
        })
        .expect("legacy media send must expose optimistic persist correlation");
    harness.step(port_reply_ok(persist_corr), 1_700_000_000_001);
    assert!(harness.sink.as_slice().iter().any(|effect| matches!(
        effect,
        Effect::Http { req, .. } if req.url.ends_with("/oss/getPresignedUrl")
    )));
    assert!(!harness.sink.as_slice().iter().any(|effect| matches!(
        effect,
        Effect::Http { req, .. } if req.url.ends_with("/oss/multipart/init")
    )));
}

fn http_corr(harness: &Harness, suffix: &str) -> u64 {
    harness
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Http { corr, req } if req.url.ends_with(suffix) => Some(corr.raw()),
            _ => None,
        })
        .expect("expected multipart HTTP effect")
}

fn stage_corr(harness: &Harness, stage: &str) -> u64 {
    harness
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Persist { corr, ops } | Effect::PersistAtomic { corr, ops }
                if ops.iter().any(|op| match op {
                    StorageOp::BatchUpsert(spec) if spec.table == "pending_media" => {
                        spec.rows.iter().flatten().any(|(name, value)| {
                            name == "stage"
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
        .expect("expected multipart stage persist")
}

fn upload_effect(harness: &Harness) -> (u64, FileUploadRequest) {
    harness
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::UploadFile { corr, req } => Some((corr.raw(), req.clone())),
            _ => None,
        })
        .expect("expected multipart UploadFile effect")
}

fn upload_effects(harness: &Harness) -> Vec<(u64, FileUploadRequest)> {
    harness
        .sink
        .as_slice()
        .iter()
        .filter_map(|effect| match effect {
            Effect::UploadFile { corr, req } => Some((corr.raw(), req.clone())),
            _ => None,
        })
        .collect()
}

fn json_reply(corr: u64, body: serde_json::Value) -> helix_core::Tick {
    port_reply_with_body(corr, Bytes::from(serde_json::to_vec(&body).unwrap()))
}

fn java_reply(corr: u64, body: serde_json::Value) -> helix_core::Tick {
    port_reply_with_body(
        corr,
        enveloped(Bytes::from(serde_json::to_vec(&body).unwrap())),
    )
}

fn large_file_send_named(tmp: &str) -> helix_core::Tick {
    large_file_send_size(tmp, MULTIPART_TEST_SIZE)
}

fn large_file_send_size(tmp: &str, size: u64) -> helix_core::Tick {
    send_message_cmd_json(json!({
        "channel_id": cid(1),
        "temporary_id": tmp,
        "type": "FILE",
        "text": "",
        "props": {
            "file": {
                "mediaInput": {
                    "localPath": "/tmp/multipart.bin",
                    "fileName": "multipart.bin",
                    "contentType": "application/octet-stream",
                    "size": size,
                    "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                }
            }
        }
    }))
}

fn valid_init() -> serde_json::Value {
    json!({
        "isSuccess": true,
        "protocolVersion": 1,
        "status": "uploading",
        "uploadId": "up-1",
        "fileId": "file-1",
        "bucket": "attachment",
        "objectKey": "media/file-1",
        "uri": "media/file-1",
        "partSize": PART_SIZE,
        "size": MULTIPART_TEST_SIZE,
        "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
    })
}

fn valid_init_for(size: u64) -> serde_json::Value {
    let mut body = valid_init();
    body["size"] = json!(size);
    body
}

fn parts_reply(parts: serde_json::Value, status: &str) -> serde_json::Value {
    json!({"isSuccess": true, "uploadId": "up-1", "status": status, "parts": parts})
}

fn completed_parts_for(size: u64) -> serde_json::Value {
    completed_parts_from(size, 1)
}

fn completed_parts_from(size: u64, first_part: u64) -> serde_json::Value {
    let count = size.div_ceil(PART_SIZE);
    json!((first_part..=count)
        .map(|part_number| {
            json!({
                "partNumber": part_number,
                "size": (size - (part_number - 1) * PART_SIZE).min(PART_SIZE),
                "etag": format!("etag-{part_number}"),
            })
        })
        .collect::<Vec<_>>())
}

fn sign_reply() -> serde_json::Value {
    json!({
        "isSuccess": true,
        "uploadId": "up-1",
        "expiresAt": 1_700_000_300_000_u64,
        "parts": [
            {"partNumber": 1, "url": "https://oss.test/part/1?sig=secret", "headers": {"Content-Type": "application/octet-stream"}},
            {"partNumber": 2, "url": "https://oss.test/part/2?sig=secret", "headers": {"Content-Type": "application/octet-stream"}}
        ]
    })
}

fn sign_reply_three() -> serde_json::Value {
    sign_reply_for(3)
}

fn sign_reply_for(count: u32) -> serde_json::Value {
    let parts = (1..=count)
        .map(|part_number| {
            json!({
                "partNumber": part_number,
                "url": format!("https://oss.test/part/{part_number}?sig=secret"),
                "headers": {"Content-Type": "application/octet-stream"}
            })
        })
        .collect::<Vec<_>>();
    json!({
        "isSuccess": true,
        "uploadId": "up-1",
        "expiresAt": 1_700_000_300_000_u64,
        "parts": parts
    })
}

fn reach_first_multipart_upload(h: &mut Harness, temporary_id: &str) -> u64 {
    h.step(large_file_send_named(temporary_id), 1_700_000_000_000);
    let initial = h
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::PersistAtomic { corr, .. } => Some(corr.raw()),
            _ => None,
        })
        .expect("initial multipart persist");
    h.step(port_reply_ok(initial), 1_700_000_000_001);
    let init = http_corr(h, "/oss/multipart/init");
    h.step(java_reply(init, valid_init()), 1_700_000_000_002);
    let init_stage = stage_corr(h, "multipart_parts");
    h.step(port_reply_ok(init_stage), 1_700_000_000_003);
    let parts = http_corr(h, "/oss/multipart/parts");
    h.step(
        java_reply(
            parts,
            parts_reply(completed_parts_from(MULTIPART_TEST_SIZE, 3), "uploading"),
        ),
        1_700_000_000_004,
    );
    let sign_stage = stage_corr(h, "multipart_sign");
    h.step(port_reply_ok(sign_stage), 1_700_000_000_005);
    let sign = http_corr(h, "/oss/multipart/sign");
    h.step(java_reply(sign, sign_reply()), 1_700_000_000_006);
    upload_effect(h).0
}

/// 公开 ImModule seam 的完整 multipart barrier：本地持久化、Java init/parts/sign、
/// range 上传、ETag checkpoint、再次 parts 对账、Java complete 和稳定引用 PersistOk。
#[test]
fn multipart_lifecycle_requires_remote_parts_and_stable_complete() {
    let mut h = Harness::new(&[1]);
    h.step(
        large_file_send_named("tmp-multipart-green"),
        1_700_000_000_000,
    );
    let p1 = h
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::PersistAtomic { corr, ops } => {
                let pending_row = ops.iter().any(|op| matches!(op,
                    StorageOp::BatchUpsert(spec) if spec.table == "pending_media"
                        && spec.rows.iter().flatten().any(|(name, value)| name == "upload_id" && matches!(value, SqlValue::Text(value) if value.is_empty()))
                ));
                let session_row = ops.iter().any(|op| matches!(op,
                    StorageOp::BatchUpsert(spec) if spec.table == "media_upload_session"));
                assert!(pending_row && session_row, "initial journal must contain old recovery row plus normalized session");
                Some(corr.raw())
            }
            _ => None,
        })
        .expect("initial multipart PersistAtomic");
    h.step(port_reply_ok(p1), 1_700_000_000_001);

    let init = http_corr(&h, "/oss/multipart/init");
    h.step(java_reply(init, valid_init()), 1_700_000_000_002);
    let stage = stage_corr(&h, "multipart_parts");
    h.step(port_reply_ok(stage), 1_700_000_000_003);

    let parts = http_corr(&h, "/oss/multipart/parts");
    h.step(
        java_reply(
            parts,
            parts_reply(completed_parts_from(MULTIPART_TEST_SIZE, 3), "uploading"),
        ),
        1_700_000_000_004,
    );
    let sign_stage = stage_corr(&h, "multipart_sign");
    h.step(port_reply_ok(sign_stage), 1_700_000_000_005);
    let sign = http_corr(&h, "/oss/multipart/sign");
    h.step(java_reply(sign, sign_reply()), 1_700_000_000_006);

    let uploads = upload_effects(&h);
    assert_eq!(
        uploads.len(),
        2,
        "a sign batch fills the two available part slots"
    );
    let (part_one, request_one) = uploads[0].clone();
    let (part_two, request_two) = uploads[1].clone();
    assert_eq!(request_one.size, Some(MULTIPART_TEST_SIZE));
    assert_eq!(
        request_one
            .range()
            .map(|range| (range.offset, range.length)),
        Some((0, PART_SIZE))
    );
    assert_eq!(
        request_two
            .range()
            .map(|range| (range.offset, range.length)),
        Some((PART_SIZE, PART_SIZE))
    );
    assert_eq!(request_two.size, Some(MULTIPART_TEST_SIZE));

    // Complete the tail first. Each ETag gets its own atomic checkpoint and
    // no Java complete can be emitted while the other slot remains in flight.
    h.step(
        json_reply(
            part_two,
            json!({"objectKey":"media/file-1", "url":"media/file-1", "etag":"etag-2"}),
        ),
        1_700_000_000_007,
    );
    let tail_checkpoint = stage_corr(&h, "multipart_parts");
    h.step(port_reply_ok(tail_checkpoint), 1_700_000_000_008);
    assert!(!h.sink.as_slice().iter().any(|effect| matches!(effect, Effect::Http { req, .. } if req.url.ends_with("/oss/multipart/complete"))));

    h.step(
        json_reply(
            part_one,
            json!({"objectKey":"media/file-1", "url":"media/file-1", "etag":"etag-1"}),
        ),
        1_700_000_000_009,
    );
    let head_checkpoint = stage_corr(&h, "multipart_parts");
    h.step(port_reply_ok(head_checkpoint), 1_700_000_000_010);
    let requery_stage = stage_corr(&h, "multipart_parts");
    h.step(port_reply_ok(requery_stage), 1_700_000_000_011);

    let parts = http_corr(&h, "/oss/multipart/parts");
    h.step(
        java_reply(
            parts,
            parts_reply(completed_parts_for(MULTIPART_TEST_SIZE), "completed"),
        ),
        1_700_000_000_012,
    );
    let complete_stage = stage_corr(&h, "multipart_complete");
    h.step(port_reply_ok(complete_stage), 1_700_000_000_013);
    let complete = http_corr(&h, "/oss/multipart/complete");
    h.step(
        java_reply(
            complete,
            json!({
                "isSuccess": true,
                "status": "completed",
                "uploadId": "up-1",
                "fileId": "file-1",
                "bucket": "attachment",
                "objectKey": "media/file-1",
                "uri": "media/file-1",
                "size": MULTIPART_TEST_SIZE,
                "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
            }),
        ),
        1_700_000_000_014,
    );
    let completion = h
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::PersistAtomic { corr, ops } if ops.iter().any(|op| matches!(op, StorageOp::BatchDelete(spec) if spec.table == "media_upload_session")) => Some(corr.raw()),
            _ => None,
        })
        .expect("complete must atomically persist stable reference and delete normalized checkpoint");
    h.step(port_reply_ok(completion), 1_700_000_000_015);
    assert!(h.sink.as_slice().iter().any(
        |effect| matches!(effect, Effect::Http { req, .. } if req.url.ends_with("/posts/create"))
    ));
}

/// `status=completed` 是 Java/OSS 的远端事实提示，不是本地成功屏障。
/// 即使 parts 查询返回空列表，也必须继续 Java complete 校验，不能重新签名或上传，
/// 更不能在 complete 的 PersistOk 之前推进 posts/create。
#[test]
fn completed_remote_session_validates_complete_without_resigning_or_uploading() {
    let mut h = Harness::new(&[1]);
    h.step(
        large_file_send_named("tmp-multipart-completed"),
        1_700_000_000_000,
    );
    let initial = h
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::PersistAtomic { corr, .. } => Some(corr.raw()),
            _ => None,
        })
        .expect("initial multipart persist");
    h.step(port_reply_ok(initial), 1_700_000_000_001);

    let init = http_corr(&h, "/oss/multipart/init");
    let mut init_body = valid_init();
    init_body["status"] = json!("completed");
    h.step(java_reply(init, init_body), 1_700_000_000_002);
    let init_stage = stage_corr(&h, "multipart_parts");
    h.step(port_reply_ok(init_stage), 1_700_000_000_003);

    let parts = http_corr(&h, "/oss/multipart/parts");
    h.step(
        java_reply(parts, parts_reply(json!([]), "completed")),
        1_700_000_000_004,
    );
    let complete_stage = stage_corr(&h, "multipart_complete");
    h.step(port_reply_ok(complete_stage), 1_700_000_000_005);

    assert!(h
        .sink
        .as_slice()
        .iter()
        .any(|effect| matches!(effect, Effect::Http { req, .. } if req.url.ends_with("/oss/multipart/complete"))));
    assert!(!h.sink.as_slice().iter().any(|effect| matches!(
        effect,
        Effect::Http { req, .. } if req.url.ends_with("/oss/multipart/sign")
    )));
    assert!(!h
        .sink
        .as_slice()
        .iter()
        .any(|effect| matches!(effect, Effect::UploadFile { .. })));
}

#[test]
fn multipart_part_checkpoint_is_atomic_and_persist_error_stops_the_chain() {
    let mut h = Harness::new(&[1]);
    let part = reach_first_multipart_upload(&mut h, "tmp-multipart-persist-error");
    h.step(
        json_reply(
            part,
            json!({"objectKey":"media/file-1", "url":"media/file-1", "etag":"etag-1"}),
        ),
        1_700_000_000_007,
    );

    let checkpoint = h
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::PersistAtomic { corr, ops } => {
                let part_rows = ops.iter().find_map(|op| match op {
                    StorageOp::BatchUpsert(spec) if spec.table == "media_upload_part" => {
                        Some(spec.rows.len())
                    }
                    _ => None,
                });
                assert_eq!(
                    part_rows,
                    Some(1),
                    "one UploadFile reply writes one part row"
                );
                Some(corr.raw())
            }
            _ => None,
        })
        .expect("ETag checkpoint must use PersistAtomic");
    h.step(port_reply_err(checkpoint), 1_700_000_000_008);

    assert!(!h.sink.as_slice().iter().any(|effect| match effect {
        Effect::UploadFile { .. } => true,
        Effect::Http { req, .. } => {
            req.url.ends_with("/oss/multipart/parts")
                || req.url.ends_with("/oss/multipart/sign")
                || req.url.ends_with("/oss/multipart/complete")
        }
        _ => false,
    }));
}

#[test]
fn multipart_first_batch_has_at_most_three_ranges_and_handles_out_of_order_etag_ack() {
    let size = MULTIPART_TEST_SIZE;
    let mut h = Harness::new(&[1]);
    h.step(
        large_file_send_size("tmp-multipart-three", size),
        1_700_000_000_000,
    );
    let initial = h
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::PersistAtomic { corr, .. } => Some(corr.raw()),
            _ => None,
        })
        .expect("initial multipart persist");
    h.step(port_reply_ok(initial), 1_700_000_000_001);

    let init = http_corr(&h, "/oss/multipart/init");
    h.step(java_reply(init, valid_init_for(size)), 1_700_000_000_002);
    let init_stage = stage_corr(&h, "multipart_parts");
    h.step(port_reply_ok(init_stage), 1_700_000_000_003);
    let parts = http_corr(&h, "/oss/multipart/parts");
    h.step(
        java_reply(
            parts,
            parts_reply(completed_parts_from(size, 4), "uploading"),
        ),
        1_700_000_000_004,
    );
    let sign_stage = stage_corr(&h, "multipart_sign");
    h.step(port_reply_ok(sign_stage), 1_700_000_000_005);
    let sign = http_corr(&h, "/oss/multipart/sign");
    h.step(java_reply(sign, sign_reply_three()), 1_700_000_000_006);

    let uploads = upload_effects(&h);
    assert_eq!(
        uploads.len(),
        3,
        "the first sign batch is bounded by three slots"
    );
    let ranges = uploads
        .iter()
        .map(|(_, request)| request.range().map(|range| (range.offset, range.length)))
        .collect::<Vec<_>>();
    assert_eq!(
        ranges,
        vec![
            Some((0, PART_SIZE)),
            Some((PART_SIZE, PART_SIZE)),
            Some((PART_SIZE * 2, PART_SIZE)),
        ]
    );

    // ACK in reverse order and checkpoint each one before the next stage.
    for (index, (corr, _)) in uploads.iter().rev().enumerate() {
        h.step(
            json_reply(
                *corr,
                json!({
                    "objectKey":"media/file-1",
                    "url":"media/file-1",
                    "etag":format!("etag-{}", 3 - index)
                }),
            ),
            1_700_000_000_007 + index as u64 * 2,
        );
        let checkpoint = stage_corr(&h, "multipart_parts");
        h.step(
            port_reply_ok(checkpoint),
            1_700_000_000_008 + index as u64 * 2,
        );
        if index < 2 {
            assert!(!h.sink.as_slice().iter().any(|effect| matches!(
                effect,
                Effect::Http { req, .. } if req.url.ends_with("/oss/multipart/complete")
            )));
        }
    }

    let requery_stage = stage_corr(&h, "multipart_parts");
    h.step(port_reply_ok(requery_stage), 1_700_000_000_014);
    assert!(h
        .sink
        .as_slice()
        .iter()
        .any(|effect| matches!(effect, Effect::Http { req, .. } if req.url.ends_with("/oss/multipart/parts"))));
}

/// Java sign 的单请求上限是 8 片；大文件的其余缺片必须在该批完成后
/// 重新 parts 对账，而不能把最多 128 个 partNumbers 一次送入请求。
#[test]
fn multipart_sign_batches_missing_parts_in_groups_of_eight() {
    let size = MULTIPART_TEST_SIZE;
    let mut h = Harness::new(&[1]);
    h.step(
        large_file_send_size("tmp-multipart-sign-batch", size),
        1_700_000_000_000,
    );
    let initial = h
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::PersistAtomic { corr, .. } => Some(corr.raw()),
            _ => None,
        })
        .expect("initial multipart persist");
    h.step(port_reply_ok(initial), 1_700_000_000_001);
    let init = http_corr(&h, "/oss/multipart/init");
    h.step(java_reply(init, valid_init_for(size)), 1_700_000_000_002);
    let init_stage = stage_corr(&h, "multipart_parts");
    h.step(port_reply_ok(init_stage), 1_700_000_000_003);
    let parts = http_corr(&h, "/oss/multipart/parts");
    h.step(
        java_reply(parts, parts_reply(json!([]), "uploading")),
        1_700_000_000_004,
    );
    let sign_stage = stage_corr(&h, "multipart_sign");
    h.step(port_reply_ok(sign_stage), 1_700_000_000_005);

    let body = h
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Http { req, .. } if req.url.ends_with("/oss/multipart/sign") => req
                .body
                .as_ref()
                .and_then(|body| serde_json::from_slice::<serde_json::Value>(body).ok()),
            _ => None,
        })
        .expect("multipart sign request");
    assert_eq!(
        body["partNumbers"].as_array().map(Vec::len),
        Some(8),
        "Java sign request must be bounded to eight parts"
    );
    assert_eq!(body["partNumbers"], json!([1, 2, 3, 4, 5, 6, 7, 8]));
}

/// 单片 EOF 只代表 range 已读完；消息进度仍须停在 100% 以下，直到
/// ETag checkpoint、Java complete 和稳定引用 PersistAtomic 全部成功。
#[test]
fn multipart_progress_does_not_emit_full_percent_before_complete_barrier() {
    let mut h = Harness::new(&[1]);
    let upload_corr = reach_first_multipart_upload(&mut h, "tmp-multipart-progress");
    h.step(
        Tick::PortProgress {
            corr: Correlation::from_raw(upload_corr),
            progress: FileUploadProgress {
                completed_bytes: PART_SIZE,
                total_bytes: PART_SIZE,
            },
        },
        1_700_000_000_007,
    );

    let persisted_percent = h
        .sink
        .as_slice()
        .iter()
        .rev()
        .find_map(|effect| match effect {
            Effect::Persist { ops, .. } => ops.iter().find_map(|op| match op {
                StorageOp::BatchUpdate(spec) if spec.table == "message" => {
                    spec.patch.iter().find_map(|(column, value)| {
                        if column != "upload_progress_percent" {
                            return None;
                        }
                        match value {
                            SqlValue::Integer(percent) => Some(*percent),
                            _ => None,
                        }
                    })
                }
                _ => None,
            }),
            _ => None,
        });
    assert!(persisted_percent.is_some_and(|percent| percent < 100));
}

/// 单片 timeout 后必须先等待同一批次的其它 UploadFile corr 终止；
/// 迟到的 sibling 回包只能 no-op，所有旧 PUT 结束后才由 timer 重新 init+parts 对账。
#[test]
fn multipart_upload_timeout_revokes_siblings_before_reconciliation() {
    let mut h = Harness::new(&[1]);
    let _ = reach_first_multipart_upload(&mut h, "tmp-multipart-timeout");
    let uploads = upload_effects(&h);
    assert_eq!(uploads.len(), 2);
    let first = uploads[0].0;
    let sibling = uploads[1].0;

    let timeout_effects = h.step(
        Tick::PortReply {
            corr: Correlation::from_raw(first),
            outcome: PortOutcome::Err(PortError::Timeout),
        },
        1_700_000_000_007,
    );
    assert!(!timeout_effects
        .iter()
        .any(|effect| matches!(effect, common::EffectKind::ScheduleTimer(_))));

    let late_effects = h.step(
        json_reply(
            sibling,
            json!({"objectKey":"media/file-1", "url":"media/file-1", "etag":"late"}),
        ),
        1_700_000_000_008,
    );
    let timer = late_effects
        .iter()
        .find_map(|effect| match effect {
            common::EffectKind::ScheduleTimer(timer) => Some(*timer),
            _ => None,
        })
        .expect("retry timer waits until every old UploadFile corr terminates");
    assert!(!late_effects.iter().any(|effect| {
        matches!(
            effect,
            common::EffectKind::UploadFile(_)
                | common::EffectKind::Http(_)
                | common::EffectKind::Emit
        )
    }));
    h.step(timer_tick(timer), 1_700_000_000_100);
    assert!(h.sink.as_slice().iter().any(|effect| matches!(
        effect,
        Effect::Http { req, .. } if req.url.ends_with("/oss/multipart/init")
    )));
}

#[test]
fn multipart_drain_waits_for_checkpoint_persist_after_old_puts_reply() {
    let mut h = Harness::new(&[1]);
    let _ = reach_first_multipart_upload(&mut h, "tmp-multipart-persist-drain");
    let uploads = upload_effects(&h);
    assert_eq!(uploads.len(), 2);
    let first = uploads[0].0;
    let sibling = uploads[1].0;

    h.step(
        json_reply(
            first,
            json!({"objectKey":"media/file-1", "url":"media/file-1", "etag":"first"}),
        ),
        1_700_000_000_007,
    );
    let checkpoint = stage_corr(&h, "multipart_parts");
    let timeout_effects = h.step(
        Tick::PortReply {
            corr: Correlation::from_raw(sibling),
            outcome: PortOutcome::Err(PortError::Timeout),
        },
        1_700_000_000_008,
    );
    assert!(!timeout_effects
        .iter()
        .any(|effect| matches!(effect, common::EffectKind::ScheduleTimer(_))));

    let checkpoint_effects = h.step(port_reply_ok(checkpoint), 1_700_000_000_009);
    assert!(checkpoint_effects
        .iter()
        .any(|effect| matches!(effect, common::EffectKind::ScheduleTimer(_))));
    assert!(!checkpoint_effects.iter().any(|effect| {
        matches!(
            effect,
            common::EffectKind::UploadFile(_)
                | common::EffectKind::Http(_)
                | common::EffectKind::Emit
        )
    }));
}

/// 永久失败也必须等旧 sibling PUT 回灌后再落 UnSend，避免用户立即重试
/// 与仍在 OSS 执行的旧 range 并发。
#[test]
fn multipart_terminal_failure_drains_siblings_before_unsend() {
    let mut h = Harness::new(&[1]);
    let _ = reach_first_multipart_upload(&mut h, "tmp-multipart-terminal-drain");
    let uploads = upload_effects(&h);
    assert_eq!(uploads.len(), 2);
    let first = uploads[0].0;
    let sibling = uploads[1].0;

    let first_effects = h.step(
        json_reply(
            first,
            json!({"objectKey":"media/file-1", "url":"media/file-1"}),
        ),
        1_700_000_000_007,
    );
    assert!(!first_effects
        .iter()
        .any(|effect| matches!(effect, common::EffectKind::Persist(_))));

    let sibling_effects = h.step(
        json_reply(
            sibling,
            json!({"objectKey":"media/file-1", "url":"media/file-1", "etag":"late"}),
        ),
        1_700_000_000_008,
    );
    assert!(sibling_effects
        .iter()
        .any(|effect| matches!(effect, common::EffectKind::Persist(_))));
    assert!(!sibling_effects.iter().any(|effect| {
        matches!(
            effect,
            common::EffectKind::UploadFile(_) | common::EffectKind::Http(_)
        )
    }));
}

/// 重启扫描只恢复可信的 prepare 输入；旧 `pending_media.upload_id` 即使
/// 携带历史 fileId 也不能被当成 multipart uploadId 直接继续。
#[test]
fn multipart_restart_recovery_returns_prepare_from_legacy_journal_row() {
    let row = json!([{
        "temporary_id": "tmp-multipart-restart",
        "target_key": "file",
        "stage": "multipart_parts",
        "channel_id": cid(1),
        "local_path": "/tmp/multipart.bin",
        "file_name": "multipart.bin",
        "content_type": "application/octet-stream",
        "size": MULTIPART_TEST_SIZE,
        "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "upload_id": "legacy-file-id",
        "file_id": "legacy-file-id"
    }]);
    let bytes = serde_json::to_vec(&row).expect("journal row JSON");
    let recovered = helix_im::send::upload_props::decode_durable_scan(&bytes)
        .expect("multipart journal row must decode");
    assert!(
        matches!(recovered.as_slice(), [FailedMediaOp::Multipart(pending)] if pending.input.size == MULTIPART_TEST_SIZE)
    );
}

#[test]
fn multipart_fourth_part_waits_for_checkpoint_before_entering_a_new_slot() {
    let size = MULTIPART_TEST_SIZE;
    let mut h = Harness::new(&[1]);
    h.step(
        large_file_send_size("tmp-multipart-four", size),
        1_700_000_000_000,
    );
    let initial = h
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::PersistAtomic { corr, .. } => Some(corr.raw()),
            _ => None,
        })
        .expect("initial multipart persist");
    h.step(port_reply_ok(initial), 1_700_000_000_001);
    let init = http_corr(&h, "/oss/multipart/init");
    h.step(java_reply(init, valid_init_for(size)), 1_700_000_000_002);
    let init_stage = stage_corr(&h, "multipart_parts");
    h.step(port_reply_ok(init_stage), 1_700_000_000_003);
    let parts = http_corr(&h, "/oss/multipart/parts");
    h.step(
        java_reply(
            parts,
            parts_reply(completed_parts_from(size, 5), "uploading"),
        ),
        1_700_000_000_004,
    );
    let sign_stage = stage_corr(&h, "multipart_sign");
    h.step(port_reply_ok(sign_stage), 1_700_000_000_005);
    let sign = http_corr(&h, "/oss/multipart/sign");
    h.step(java_reply(sign, sign_reply_for(4)), 1_700_000_000_006);

    let initial_uploads = upload_effects(&h);
    assert_eq!(initial_uploads.len(), 3);
    assert_eq!(
        initial_uploads[2].1.range().map(|r| (r.offset, r.length)),
        Some((PART_SIZE * 2, PART_SIZE))
    );

    // The fourth signed URL is held in memory until the first ETag's journal
    // ACK; a PersistErr at this boundary cannot silently expose part four.
    let first = initial_uploads[0].0;
    h.step(
        json_reply(
            first,
            json!({"objectKey":"media/file-1", "url":"media/file-1", "etag":"etag-1"}),
        ),
        1_700_000_000_007,
    );
    let first_checkpoint = stage_corr(&h, "multipart_parts");
    h.step(port_reply_ok(first_checkpoint), 1_700_000_000_008);
    let next_stage = stage_corr(&h, "multipart_parts");
    assert!(!h
        .sink
        .as_slice()
        .iter()
        .any(|effect| matches!(effect, Effect::UploadFile { .. })));
    h.step(port_reply_ok(next_stage), 1_700_000_000_009);
    let fourth = upload_effects(&h);
    assert_eq!(fourth.len(), 1);
    assert_eq!(
        fourth[0].1.range().map(|r| (r.offset, r.length)),
        Some((PART_SIZE * 3, PART_SIZE))
    );

    for (index, (corr, _)) in initial_uploads
        .iter()
        .skip(1)
        .chain(fourth.iter())
        .enumerate()
    {
        h.step(
            json_reply(
                *corr,
                json!({"objectKey":"media/file-1", "url":"media/file-1", "etag":format!("etag-{}", index + 2)}),
            ),
            1_700_000_000_010 + index as u64 * 2,
        );
        let checkpoint = stage_corr(&h, "multipart_parts");
        h.step(
            port_reply_ok(checkpoint),
            1_700_000_000_011 + index as u64 * 2,
        );
    }

    let requery_stage = stage_corr(&h, "multipart_parts");
    h.step(port_reply_ok(requery_stage), 1_700_000_000_020);
    assert!(h
        .sink
        .as_slice()
        .iter()
        .any(|effect| matches!(effect, Effect::Http { req, .. } if req.url.ends_with("/oss/multipart/parts"))));
}

#[test]
fn multipart_transport_retry_reconciles_from_init_with_bounded_backoff() {
    let mut h = Harness::new(&[1]);
    h.step(
        large_file_send_named("tmp-multipart-retry"),
        1_700_000_000_000,
    );
    let initial = h
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::PersistAtomic { corr, .. } => Some(corr.raw()),
            _ => None,
        })
        .expect("initial multipart persist");
    h.step(port_reply_ok(initial), 1_700_000_000_001);

    for (attempt, expected_backoff) in [(1_u8, 100_u64), (2, 200), (3, 400)] {
        let init = http_corr(&h, "/oss/multipart/init");
        h.step(port_reply_err(init), 1_700_000_000_001 + u64::from(attempt));
        let (timer, backoff) = h
            .sink
            .as_slice()
            .iter()
            .find_map(|effect| match effect {
                Effect::ScheduleTimer { id, after_ms } => Some((id.raw(), *after_ms)),
                _ => None,
            })
            .expect("retryable transport error must schedule reconciliation");
        assert_eq!(backoff, expected_backoff);
        h.step(timer_tick(timer), 1_700_000_000_100 + u64::from(attempt));
        assert!(h
            .sink
            .as_slice()
            .iter()
            .any(|effect| matches!(effect, Effect::Http { req, .. } if req.url.ends_with("/oss/multipart/init"))));
    }

    let exhausted = http_corr(&h, "/oss/multipart/init");
    h.step(port_reply_err(exhausted), 1_700_000_000_200);
    assert!(!h
        .sink
        .as_slice()
        .iter()
        .any(|effect| matches!(effect, Effect::ScheduleTimer { .. })));
    assert!(h.sink.as_slice().iter().any(|effect| matches!(
        effect,
        Effect::Persist { .. } | Effect::PersistAtomic { .. }
    )));
}
