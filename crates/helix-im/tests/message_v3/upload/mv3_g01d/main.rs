//! MV3-G01d｜发送 FILE：stage → prepare → PUT → complete → posts/create → echo 全链。
//!
//! 线型权威：`specs/003-messagev3-gate-inventory/gates/upload/mv3-g01d/*.json`
//! （`effects-contract.json` 的 `reference_sequence`、`outbound-sample.json` 的最终
//! `props.file` 形态）。Host ingress（`im_stage_media`）已在 visual 侧落地，本模块只验证
//! Core：Angular 提交的必须是 **Host 已解析** 的可信 `mediaInput`，Core 负责编排与最终引用。

use bytes::Bytes;
use helix_core::effect::{Effect, SqlValue, StorageOp, UpsertSpec};
use helix_core::{Correlation, EffectSink, Module, Tick};
use serde_json::{json, Value};

#[path = "../../../common.rs"]
mod common;
#[path = "../../support/phase1_reference.rs"]
mod phase1_reference;

use common::{cid, enveloped, port_reply_ok, port_reply_with_body, send_message_cmd_json, Harness};

const SHA256: &str = "0000000000000000000000000000000000000000000000000000000000000001";
const LOCAL_PATH: &str = "/var/im-stage/phase1.pdf.stage";

/// Host 暂存后交给 Core 的可信媒体输入（Angular 侧只见 opaque handle）。
fn media_input() -> Value {
    json!({
        "localPath": LOCAL_PATH,
        "fileName": "phase1.pdf",
        "contentType": "application/pdf",
        "size": 4096,
        "sha256": SHA256
    })
}

/// 参考包 `inbound-sample.json` 的 FILE 发送意图（handle 已由 Host 换成 mediaInput）。
fn file_send_tick(temporary_id: &str) -> Tick {
    send_message_cmd_json(json!({
        "channel_id": cid(1),
        "temporary_id": temporary_id,
        "type": "FILE",
        "text": "",
        "props": { "file": { "mediaInput": media_input() } },
        "viewers": ["all"],
        "mentions": []
    }))
}

fn http_with_suffix(h: &Harness, suffix: &str) -> Vec<(u64, Value)> {
    h.sink
        .as_slice()
        .iter()
        .filter_map(|effect| match effect {
            Effect::Http { corr, req } if req.url.ends_with(suffix) => Some((
                corr.raw(),
                req.body
                    .as_ref()
                    .and_then(|body| serde_json::from_slice(body.as_ref()).ok())
                    .unwrap_or(Value::Null),
            )),
            _ => None,
        })
        .collect()
}

fn upload_corrs(h: &Harness) -> Vec<u64> {
    h.sink
        .as_slice()
        .iter()
        .filter_map(|effect| match effect {
            Effect::UploadFile { corr, .. } => Some(corr.raw()),
            _ => None,
        })
        .collect()
}

/// 定位本 step 中写入指定 `pending_media.stage` 的持久化 correlation。
fn stage_corr(h: &Harness, stage: &str) -> u64 {
    h.sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Persist { corr, ops } | Effect::PersistAtomic { corr, ops }
                if ops.iter().any(|op| match op {
                    StorageOp::BatchUpsert(spec) if spec.table == "pending_media" => {
                        spec.rows.iter().flatten().any(|(column, value)| {
                            column == "stage"
                                && matches!(value, SqlValue::Text(text) if text == stage)
                        })
                    }
                    _ => false,
                }) =>
            {
                Some(corr.raw())
            }
            _ => None,
        })
        .unwrap_or_else(|| panic!("missing pending_media stage={stage} persist"))
}

fn last_message_upsert(h: &Harness) -> Option<UpsertSpec> {
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
}

/// 从 message 行读回持久化 props（Storage read-back 面）。
fn persisted_props(h: &Harness) -> Value {
    let spec = last_message_upsert(h).expect("message upsert");
    let text = spec.rows[0]
        .iter()
        .find_map(|(column, value)| match value {
            SqlValue::Text(text) if column == "props" => Some(text.clone()),
            _ => None,
        })
        .expect("props column");
    serde_json::from_str(&text).expect("props json")
}

fn emitted(h: &Harness, event: &str) -> Vec<Value> {
    h.sink
        .as_slice()
        .iter()
        .filter_map(|effect| match effect {
            Effect::Emit { event: bytes } => serde_json::from_slice::<Value>(bytes.0.as_ref()).ok(),
            _ => None,
        })
        .filter(|value| value["event"] == event)
        .collect()
}

fn completion_commit_corr(h: &Harness) -> u64 {
    h.sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::PersistAtomic { corr, .. } => Some(corr.raw()),
            _ => None,
        })
        .expect("completion commit persist")
}

fn prepare_reply_outcome(data: Value) -> helix_core::tick::PortOutcome {
    match port_reply_with_body(
        0,
        enveloped(Bytes::from(
            serde_json::to_vec(&data).expect("prepare json"),
        )),
    ) {
        Tick::PortReply { outcome, .. } => outcome,
        _ => unreachable!(),
    }
}

/// Java `/oss/getPresignedUrl` 的合法票据。
///
/// 注意 `bucket/objectKey/uri` 取 **helix 现行 Java 合同**（`bucket == target.bucket()`、
/// `objectKey == media/{fileId}`、`uri == objectKey`），与参考包 outbound-sample 的
/// `bucket:"message" / uri:"/message/phase1.pdf"` 不一致 —— 已按 CONTRACT_DRIFT 上报，
/// 未经裁决不得改样本，也不能在本测试里伪造一个 Core 会拒绝的票据。
fn valid_prepare_data() -> Value {
    json!({
        "isSuccess": true,
        "fileId": "file-001",
        "bucket": "attachment",
        "method": "PUT",
        "url": "https://oss.test/media/file-001?signature=short-lived",
        "uri": "media/file-001",
        "objectKey": "media/file-001",
        "headers": {
            "Content-Type": "application/pdf",
            "x-oss-forbid-overwrite": "true"
        }
    })
}

fn reply(h: &mut Harness, corr: u64, outcome: helix_core::tick::PortOutcome, now_ms: u64) {
    h.step(
        Tick::PortReply {
            corr: Correlation::from_raw(corr),
            outcome,
        },
        now_ms,
    );
}

/// 把一条 FILE 消息从命令一路推到「posts/create 已出站」；返回累计 posts/create 次数。
fn drive_full_chain(h: &mut Harness, temporary_id: &str, base_ms: u64) -> usize {
    let mut posts_create = 0usize;
    h.step(file_send_tick(temporary_id), base_ms);
    let p1 = stage_corr(h, "prepare");
    h.step(port_reply_ok(p1), base_ms + 1);
    let prepare_corr = http_with_suffix(h, "/oss/getPresignedUrl")[0].0;
    reply(
        h,
        prepare_corr,
        prepare_reply_outcome(valid_prepare_data()),
        base_ms + 2,
    );
    let put_stage = stage_corr(h, "put");
    h.step(port_reply_ok(put_stage), base_ms + 3);
    let upload_corr = upload_corrs(h)[0];
    h.step(port_reply_ok(upload_corr), base_ms + 4);
    let complete_stage = stage_corr(h, "complete");
    h.step(port_reply_ok(complete_stage), base_ms + 5);
    posts_create += http_with_suffix(h, "/posts/create").len();
    let commit = completion_commit_corr(h);
    h.step(port_reply_ok(commit), base_ms + 6);
    posts_create += http_with_suffix(h, "/posts/create").len();
    posts_create
}

/// 证明 MV3-G01d 的 visual 目标参考包完整且未伪装为运行时证据。
#[test]
fn reference_package_is_complete() {
    phase1_reference::assert_reference_gate("MV3-G01d", "upload/mv3-g01d");
}

/// effects-contract `reference_sequence[1]`：首个 Effect 必须是 **PersistAtomic**
/// （message 骨架 + pending_media journal 同事务），且在 PersistOk 之前零 Emit / 零 HTTP /
/// 零 timer（INV-04）。
#[test]
fn first_effect_is_persist_atomic_without_emit_http_or_timer() {
    let mut h = Harness::new(&[1]);
    let effects = h.step(file_send_tick("tmp-mv3-g01d-001"), 1_700_000_000_000);

    assert!(
        matches!(effects.first(), Some(common::EffectKind::PersistAtomic(_))),
        "首个 Effect 必须是 PersistAtomic，实际 {effects:?}"
    );
    assert!(
        effects.iter().all(|effect| matches!(
            effect,
            common::EffectKind::PersistAtomic(_) | common::EffectKind::Persist(_)
        )),
        "optimistic 落库前不得出现 Emit/Http/ScheduleTimer：{effects:?}"
    );

    let tables: Vec<String> = h
        .sink
        .as_slice()
        .iter()
        .flat_map(|effect| match effect {
            Effect::PersistAtomic { ops, .. } => ops.clone(),
            _ => Vec::new(),
        })
        .filter_map(|op| match op {
            StorageOp::BatchUpsert(spec) => Some(spec.table.to_string()),
            _ => None,
        })
        .collect();
    assert!(tables.iter().any(|table| table == "message"), "{tables:?}");
    assert!(
        tables.iter().any(|table| table == "pending_media"),
        "{tables:?}"
    );
}

/// 乐观 Sending 投影必须自带逐文件与消息级绝对上传进度（INV-01：Angular 不自算）。
#[test]
fn sending_projection_carries_absolute_upload_progress() {
    let mut h = Harness::new_with_identity(&[1], "upload-viewer", "upload-company", "", "", "");
    h.step(file_send_tick("tmp-mv3-g01d-002"), 1_700_000_000_000);
    let p1 = stage_corr(&h, "prepare");
    h.step(port_reply_ok(p1), 1_700_000_000_001);

    let sending = emitted(&h, "im:post:sending");
    assert_eq!(sending.len(), 1, "PersistOk 后恰好一条 im:post:sending");
    let data = &sending[0]["data"];
    assert_eq!(data["temporaryId"], "tmp-mv3-g01d-002");
    assert_eq!(data["type"], "FILE");
    assert_eq!(data["sendStatus"], "sending");
    assert_eq!(data["props"]["file"]["uploadStatus"], "pending");
    assert_eq!(data["props"]["file"]["uploadProgress"], 0);
    assert_eq!(data["props"]["uploadProgress"], 0);
}

/// T073 / INV-01：Core 只接受 Host 已解析的 mediaInput。
/// opaque `mediaHandle` 活着进 Core ⇒ Host 绑定被绕过，必须零副作用 fail closed。
#[test]
fn unresolved_media_handle_fails_closed_without_any_effect() {
    for props in [
        json!({ "file": { "mediaHandle": "media-file-001" } }),
        json!({ "file": { "media_handle": "media-file-001", "mediaInput": media_input() } }),
        json!({ "file": { "mediaInput": media_input() }, "nested": [{ "mediaHandle": "x" }] }),
    ] {
        let mut h = Harness::new(&[1]);
        let tick = send_message_cmd_json(json!({
            "channel_id": cid(1),
            "temporary_id": "tmp-mv3-g01d-handle",
            "type": "FILE",
            "text": "",
            "props": props,
            "viewers": ["all"]
        }));
        let mut out = EffectSink::new();
        let error = h
            .module
            .handle(&tick, 1_700_000_000_000, &mut out)
            .expect_err("未解析的 mediaHandle 必须 fail closed");
        assert!(
            error.to_string().contains("media handle"),
            "错误必须点名 handle：{error}"
        );
        assert!(out.as_slice().is_empty(), "fail closed 必须零 Effect");
    }
}

/// prepare 出站只带服务端需要的元数据；本地绝对路径与 SHA 属 Host 私有，绝不上线。
#[test]
fn prepare_request_never_leaks_local_path_or_digest() {
    let mut h = Harness::new(&[1]);
    h.step(file_send_tick("tmp-mv3-g01d-003"), 1_700_000_000_000);
    let props_at_send = persisted_props(&h).to_string();
    let p1 = stage_corr(&h, "prepare");
    h.step(port_reply_ok(p1), 1_700_000_000_001);

    let prepares = http_with_suffix(&h, "/oss/getPresignedUrl");
    assert_eq!(prepares.len(), 1, "单文件恰好一次 prepare");
    let body = prepares[0].1.to_string();
    assert_eq!(prepares[0].1["fileName"], "phase1.pdf");
    assert_eq!(prepares[0].1["contentType"], "application/pdf");
    assert_eq!(prepares[0].1["size"], 4096);
    assert_eq!(prepares[0].1["bucket"], "attachment");
    for secret in [LOCAL_PATH, "mediaInput"] {
        assert!(!body.contains(secret), "prepare body 泄漏 {secret}");
        assert!(!props_at_send.contains(secret), "落库 props 泄漏 {secret}");
    }
    assert!(
        !props_at_send.contains(SHA256),
        "落库 props 泄漏 Host 私有 SHA"
    );
}

/// 全链闭合：posts/create 恰好一次，且 15s echo timer 只在它真正出站的同一步 arm（INV-04）。
#[test]
fn posts_create_and_echo_timer_arm_together_exactly_once() {
    let mut h = Harness::new(&[1]);
    let mut timers_before_post = 0usize;
    let count_timers = |effects: &[common::EffectKind]| {
        effects
            .iter()
            .filter(|effect| matches!(effect, common::EffectKind::ScheduleTimer(_)))
            .count()
    };

    let first = h.step(file_send_tick("tmp-mv3-g01d-004"), 1_700_000_000_000);
    timers_before_post += count_timers(&first);
    let p1 = stage_corr(&h, "prepare");
    let after_p1 = h.step(port_reply_ok(p1), 1_700_000_000_001);
    timers_before_post += count_timers(&after_p1);
    let prepare_corr = http_with_suffix(&h, "/oss/getPresignedUrl")[0].0;
    reply(
        &mut h,
        prepare_corr,
        prepare_reply_outcome(valid_prepare_data()),
        1_700_000_000_002,
    );
    let put_stage = stage_corr(&h, "put");
    let after_put = h.step(port_reply_ok(put_stage), 1_700_000_000_003);
    timers_before_post += count_timers(&after_put);

    let uploads = upload_corrs(&h);
    assert_eq!(uploads.len(), 1, "单文件恰好一个 UploadFile");
    assert!(
        http_with_suffix(&h, "/posts/create").is_empty(),
        "PUT 尚未回包不得发 posts/create"
    );
    assert_eq!(
        timers_before_post, 0,
        "posts/create 之前不得 arm echo timer"
    );

    let after_upload = h.step(port_reply_ok(uploads[0]), 1_700_000_000_004);
    timers_before_post += count_timers(&after_upload);
    let complete_stage = stage_corr(&h, "complete");
    let after_complete = h.step(port_reply_ok(complete_stage), 1_700_000_000_005);
    timers_before_post += count_timers(&after_complete);
    assert!(
        http_with_suffix(&h, "/posts/create").is_empty(),
        "complete 阶段持久化未 ACK 前不得发 posts/create"
    );
    assert_eq!(
        timers_before_post, 0,
        "posts/create 之前不得 arm echo timer"
    );
    let commit = completion_commit_corr(&h);

    let final_effects = h.step(port_reply_ok(commit), 1_700_000_000_006);
    assert_eq!(
        http_with_suffix(&h, "/posts/create").len(),
        1,
        "完成提交 ACK 后恰好一次 posts/create"
    );
    assert_eq!(
        count_timers(&final_effects),
        1,
        "echo timer 必须与 posts/create 同步 arm"
    );
}

/// 最终 `props.file` 必须是服务端引用形态（参考包 outbound-sample）：
/// bucket / id / uri / name / contentType / size 齐备，且不含任何 Host 私有或短期凭据字段。
#[test]
fn final_file_projection_is_server_owned_and_free_of_credentials() {
    let mut h = Harness::new(&[1]);
    let posts_create = drive_full_chain(&mut h, "tmp-mv3-g01d-005", 1_700_000_000_000);
    assert_eq!(posts_create, 1);

    let body = http_with_suffix(&h, "/posts/create")[0].1.clone();
    let file = &body["props"]["file"];
    assert_eq!(file["bucket"], "attachment");
    assert_eq!(file["id"], "file-001");
    assert_eq!(file["uri"], "media/file-001");
    assert_eq!(file["name"], "phase1.pdf");
    assert_eq!(file["contentType"], "application/pdf");
    assert_eq!(file["size"], 4096);
    assert_eq!(file["uploadStatus"], "completed");
    assert_eq!(file["uploadProgress"], 100);
    assert_eq!(
        body["props"]["uploadProgress"], 100,
        "消息级进度必须收口到 100"
    );

    for forbidden in [
        "mediaHandle",
        "media_handle",
        "mediaInput",
        "localPath",
        "url",
        "publicUrl",
        "headers",
        "objectKey",
        "method",
    ] {
        assert!(
            file.get(forbidden).is_none(),
            "最终 props.file 不得携带 {forbidden}：{file}"
        );
    }
    let rendered = body.to_string();
    for secret in [
        LOCAL_PATH,
        "signature=short-lived",
        "x-oss-forbid-overwrite",
    ] {
        assert!(
            !rendered.contains(secret),
            "posts/create body 泄漏 {secret}"
        );
    }
}

/// 幂等：迟到 / 重放的 UploadFile PortReply 不得再触发一次上传或第二条 posts/create。
#[test]
fn replayed_upload_reply_is_idempotent() {
    let mut h = Harness::new(&[1]);

    h.step(file_send_tick("tmp-mv3-g01d-006"), 1_700_000_001_000);
    let p1 = stage_corr(&h, "prepare");
    h.step(port_reply_ok(p1), 1_700_000_001_001);
    let prepare_corr = http_with_suffix(&h, "/oss/getPresignedUrl")[0].0;
    reply(
        &mut h,
        prepare_corr,
        prepare_reply_outcome(valid_prepare_data()),
        1_700_000_001_002,
    );
    let put_stage = stage_corr(&h, "put");
    h.step(port_reply_ok(put_stage), 1_700_000_001_003);
    let upload_corr = upload_corrs(&h)[0];
    h.step(port_reply_ok(upload_corr), 1_700_000_001_004);
    let complete_stage = stage_corr(&h, "complete");
    h.step(port_reply_ok(complete_stage), 1_700_000_001_005);
    let commit = completion_commit_corr(&h);
    h.step(port_reply_ok(commit), 1_700_000_001_006);
    assert_eq!(http_with_suffix(&h, "/posts/create").len(), 1);

    let replay = h.step(port_reply_ok(upload_corr), 1_700_000_001_007);
    assert!(
        http_with_suffix(&h, "/posts/create").is_empty(),
        "重放 UploadFile 回包不得产生第二次 posts/create"
    );
    assert!(
        replay
            .iter()
            .all(|effect| !matches!(effect, common::EffectKind::UploadFile(_))),
        "重放不得重新上传：{replay:?}"
    );
}
