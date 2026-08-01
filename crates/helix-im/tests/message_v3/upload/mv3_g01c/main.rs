//! MV3-G01c｜发送多图 IMAGE：多目标、逐文件绝对进度、按总字节加权的消息级进度、
//! 部分失败与选择性重试。
//!
//! 线型权威：`specs/003-messagev3-gate-inventory/gates/upload/mv3-g01c/*.json`。
//! 专项断言（helix-gate-test-matrix §3.3）：
//! ① `props.files[0]` 与 `props.files[1]` 的绝对进度**各自独立推进**；
//! ② 整条消息进度按 `(1024*p0 + 2048*p1) / 3072` **加权**，不是按文件数取平均；
//! ③ 任一目标失败 ⇒ 整条消息**零 posts/create**。

use bytes::Bytes;
use helix_core::effect::{Effect, SqlValue, StorageOp, UpsertSpec};
use helix_core::{Correlation, Tick};
use helix_im::send::upload_props::UploadTarget;
use serde_json::{json, Value};

#[path = "../../../common.rs"]
mod common;
#[path = "../../support/phase1_reference.rs"]
mod phase1_reference;

use common::{
    cid, enveloped, port_reply_err, port_reply_ok, port_reply_with_body, send_message_cmd_json,
    Harness,
};

const SHA_A: &str = "0000000000000000000000000000000000000000000000000000000000000001";
const SHA_B: &str = "0000000000000000000000000000000000000000000000000000000000000002";
const PATH_A: &str = "/var/im-stage/first.png.stage";
const PATH_B: &str = "/var/im-stage/second.jpg.stage";
const SIZE_A: u64 = 1024;
const SIZE_B: u64 = 2048;

fn image_input(path: &str, name: &str, content_type: &str, size: u64, sha: &str) -> Value {
    json!({
        "localPath": path,
        "fileName": name,
        "contentType": content_type,
        "size": size,
        "sha256": sha
    })
}

/// 参考包 `inbound-sample.json` 的多图意图（两个 handle 已由 Host 换成 mediaInput）。
fn two_image_tick(temporary_id: &str) -> Tick {
    send_message_cmd_json(json!({
        "channel_id": cid(1),
        "temporary_id": temporary_id,
        "type": "IMAGE",
        "text": "两张图片",
        "props": {
            "imagePosition": "bottom",
            "files": [
                { "mediaInput": image_input(PATH_A, "first.png", "image/png", SIZE_A, SHA_A) },
                { "mediaInput": image_input(PATH_B, "second.jpg", "image/jpeg", SIZE_B, SHA_B) }
            ]
        },
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

fn stage_corrs(h: &Harness, stage: &str) -> Vec<u64> {
    h.sink
        .as_slice()
        .iter()
        .filter_map(|effect| match effect {
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
        .collect()
}

fn persist_atomic_corrs(h: &Harness) -> Vec<u64> {
    h.sink
        .as_slice()
        .iter()
        .filter_map(|effect| match effect {
            Effect::PersistAtomic { corr, .. } => Some(corr.raw()),
            _ => None,
        })
        .collect()
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

fn reply(h: &mut Harness, corr: u64, outcome: helix_core::tick::PortOutcome, now_ms: u64) {
    h.step(
        Tick::PortReply {
            corr: Correlation::from_raw(corr),
            outcome,
        },
        now_ms,
    );
}

fn prepare_outcome(file_id: &str, content_type: &str) -> helix_core::tick::PortOutcome {
    // bucket / objectKey / uri 取 helix 现行 Java 合同：bucket==picture、
    // objectKey==media/{fileId}、uri==objectKey（与参考包 outbound-sample 的
    // bucket:"message" 冲突，已作为 CONTRACT_DRIFT 上报，未裁决前不改样本也不伪造票据）。
    let data = json!({
        "isSuccess": true,
        "fileId": file_id,
        "bucket": "picture",
        "method": "PUT",
        "url": format!("https://oss.test/media/{file_id}?signature=short-lived"),
        "uri": format!("media/{file_id}"),
        "objectKey": format!("media/{file_id}"),
        "headers": {
            "Content-Type": content_type,
            "x-oss-forbid-overwrite": "true"
        }
    });
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

/// 单个目标从 prepare 票据回包一路走到「completion 已提交」。
///
/// `prepare_corr` 是该目标的 `/oss/getPresignedUrl` correlation。
///
/// 返回 completion 原子提交里真正落库的 props（Storage read-back 面）。
fn settle_target(
    h: &mut Harness,
    prepare_corr: u64,
    file_id: &str,
    content_type: &str,
    t: u64,
) -> Value {
    reply(h, prepare_corr, prepare_outcome(file_id, content_type), t);
    let put_stage = stage_corrs(h, "put");
    assert_eq!(put_stage.len(), 1, "prepare 回包后恰好一次 put 阶段持久化");
    h.step(port_reply_ok(put_stage[0]), t + 1);
    let uploads = upload_corrs(h);
    assert_eq!(uploads.len(), 1, "put 阶段落库后恰好一个 UploadFile");
    h.step(port_reply_ok(uploads[0]), t + 2);
    let complete_stage = stage_corrs(h, "complete");
    assert_eq!(
        complete_stage.len(),
        1,
        "PUT 成功后恰好一次 complete 阶段落库"
    );
    h.step(port_reply_ok(complete_stage[0]), t + 3);
    let commit = persist_atomic_corrs(h);
    assert_eq!(commit.len(), 1, "complete 阶段应只提交一次原子写");
    let committed_props = persisted_props(h);
    h.step(port_reply_ok(commit[0]), t + 4);
    committed_props
}

/// 推进到「两个 prepare 已出站」，返回按 files 顺序排列的 prepare correlation。
fn start_two_image_send(h: &mut Harness, temporary_id: &str) -> Vec<(u64, Value)> {
    h.step(two_image_tick(temporary_id), 1_700_000_000_000);
    let p1 = stage_corrs(h, "prepare");
    assert_eq!(p1.len(), 1, "两图的 journal 必须与 message 同一次原子写");
    h.step(port_reply_ok(p1[0]), 1_700_000_000_001);
    let prepares = http_with_suffix(h, "/oss/getPresignedUrl");
    assert_eq!(prepares.len(), 2, "两图必须各自独立 prepare");
    prepares
}

/// 证明 MV3-G01c 的 visual 目标参考包完整且未伪装为运行时证据。
#[test]
fn reference_package_is_complete() {
    phase1_reference::assert_reference_gate("MV3-G01c", "upload/mv3-g01c");
}

/// 多图必须是**一条消息、多个上传目标**：两次 prepare 各自携带自己的文件元数据与
/// 互不相同的 clientUploadId（否则 Java 侧会把两张图折叠成同一张票据）。
#[test]
fn two_images_produce_two_independent_prepare_targets() {
    let mut h = Harness::new(&[1]);
    let prepares = start_two_image_send(&mut h, "tmp-mv3-g01c-001");

    let first = &prepares[0].1;
    let second = &prepares[1].1;
    assert_eq!(first["fileName"], "first.png");
    assert_eq!(first["contentType"], "image/png");
    assert_eq!(first["size"], SIZE_A);
    assert_eq!(second["fileName"], "second.jpg");
    assert_eq!(second["contentType"], "image/jpeg");
    assert_eq!(second["size"], SIZE_B);
    assert_ne!(
        first["clientUploadId"], second["clientUploadId"],
        "两个目标的 clientUploadId 必须可区分"
    );
    for body in [first, second] {
        assert_eq!(body["bucket"], "picture");
        let rendered = body.to_string();
        for secret in [PATH_A, PATH_B] {
            assert!(!rendered.contains(secret), "prepare 泄漏本地路径 {secret}");
        }
    }
    assert!(
        http_with_suffix(&h, "/posts/create").is_empty(),
        "上传未完成不得发 posts/create"
    );
}

/// 乐观 Sending 首帧即带齐**逐文件**绝对进度与消息级加权进度（INV-01）。
#[test]
fn sending_projection_carries_per_file_pending_progress() {
    let mut h = Harness::new(&[1]);
    start_two_image_send(&mut h, "tmp-mv3-g01c-002");

    let sending = emitted(&h, "im:post:sending");
    assert_eq!(sending.len(), 1);
    let props = &sending[0]["data"]["props"];
    assert_eq!(props["imagePosition"], "bottom");
    for index in 0..2 {
        assert_eq!(props["files"][index]["uploadStatus"], "pending");
        assert_eq!(props["files"][index]["uploadProgress"], 0);
    }
    assert_eq!(props["uploadProgress"], 0, "消息级加权进度首帧为 0");
    assert_eq!(props["files"][0]["size"], SIZE_A);
    assert_eq!(props["files"][1]["size"], SIZE_B);
}

/// 专项断言②：消息级进度按 **总字节加权**，不是按文件数取平均。
///
/// 1024B@100% + 2048B@0%  → 33（按文件数平均会是 50）
/// 1024B@50%  + 2048B@25% → 33（按文件数平均会是 37）
#[test]
fn message_progress_is_weighted_by_total_bytes_not_file_count() {
    let base = json!({
        "imagePosition": "bottom",
        "files": [
            {"size": SIZE_A, "uploadProgress": 0, "uploadStatus": "pending"},
            {"size": SIZE_B, "uploadProgress": 0, "uploadStatus": "pending"}
        ]
    });

    let mut props = base.clone();
    UploadTarget::RichImage { index: 0 }
        .apply_upload_state(&mut props, "completed", Some(100))
        .expect("target 0 update");
    assert_eq!(
        UploadTarget::message_upload_progress(&props).expect("weighted"),
        33,
        "小图跑满不得把整条消息拉到 50%"
    );

    let mut props = base.clone();
    UploadTarget::RichImage { index: 0 }
        .apply_upload_state(&mut props, "uploading", Some(50))
        .expect("target 0 update");
    UploadTarget::RichImage { index: 1 }
        .apply_upload_state(&mut props, "uploading", Some(25))
        .expect("target 1 update");
    assert_eq!(
        UploadTarget::message_upload_progress(&props).expect("weighted"),
        33
    );

    // 反向边界：大图跑满、小图未动 —— 加权应显著高于 50。
    let mut props = base.clone();
    UploadTarget::RichImage { index: 1 }
        .apply_upload_state(&mut props, "completed", Some(100))
        .expect("target 1 update");
    assert_eq!(
        UploadTarget::message_upload_progress(&props).expect("weighted"),
        66
    );

    // 全部完成才是 100。
    UploadTarget::RichImage { index: 0 }
        .apply_upload_state(&mut props, "completed", Some(100))
        .expect("target 0 update");
    assert_eq!(
        UploadTarget::message_upload_progress(&props).expect("weighted"),
        100
    );
}

/// T076 性能约束：单文件状态更新是 **O(1)**，不得退化成整表重算。
///
/// 手法：把兄弟节点毒化成非对象。任何「顺带整表扫描/重算」的实现都会在读到毒节点时炸掉；
/// 真正 O(1) 的实现只碰目标节点，因此必须成功，且毒节点逐字节不变。
/// 同时反证毒化确实可被检测：O(n) 的消息级投影读到它必须报错。
#[test]
fn per_file_update_is_o1_and_never_rescans_siblings() {
    let mut props = json!({
        "imagePosition": "bottom",
        "files": [
            {"size": SIZE_A, "uploadProgress": 0, "uploadStatus": "pending"},
            "poisoned-sibling-not-an-object"
        ]
    });

    UploadTarget::RichImage { index: 0 }
        .apply_upload_state(&mut props, "uploading", Some(40))
        .expect("单文件更新不得读取兄弟节点");
    assert_eq!(props["files"][0]["uploadProgress"], 40);
    assert_eq!(props["files"][0]["uploadStatus"], "uploading");
    assert_eq!(
        props["files"][1], "poisoned-sibling-not-an-object",
        "兄弟节点必须逐字节不变"
    );

    UploadTarget::message_upload_progress(&props)
        .expect_err("O(n) 消息级投影必须能看见毒节点——否则上面的成功不构成 O(1) 证据");

    // 兄弟节点合法时，单文件更新同样不得改动它。
    let mut props = json!({
        "imagePosition": "bottom",
        "files": [
            {"size": SIZE_A, "uploadProgress": 0, "uploadStatus": "pending"},
            {"size": SIZE_B, "uploadProgress": 70, "uploadStatus": "uploading", "keep": {"deep": [1, 2]}}
        ]
    });
    let sibling_before = props["files"][1].clone();
    UploadTarget::RichImage { index: 0 }
        .apply_upload_state(&mut props, "completed", Some(100))
        .expect("target 0 update");
    assert_eq!(props["files"][1], sibling_before);
}

/// 专项断言①：第一张图完成只推进它自己的绝对进度，第二张原地不动；
/// 消息级进度同步收敛到加权值，并且**一张都不缺**之前零 posts/create。
#[test]
fn first_completion_advances_only_its_own_file_and_weighted_total() {
    let mut h = Harness::new(&[1]);
    let prepares = start_two_image_send(&mut h, "tmp-mv3-g01c-003");
    let props = settle_target(
        &mut h,
        prepares[0].0,
        "file-image-001",
        "image/png",
        1_700_000_000_010,
    );

    assert_eq!(props["files"][0]["uploadStatus"], "completed");
    assert_eq!(props["files"][0]["uploadProgress"], 100);
    assert_eq!(props["files"][0]["id"], "file-image-001");
    assert_eq!(
        props["files"][1]["uploadStatus"], "pending",
        "未完成的目标状态不得被兄弟目标带跑"
    );
    assert_eq!(props["files"][1]["uploadProgress"], 0);
    assert!(
        props["files"][1].get("id").is_none(),
        "未完成目标不得凭空获得服务端 id"
    );
    assert_eq!(
        props["uploadProgress"], 33,
        "(1024*100 + 2048*0)/3072 = 33，按文件数平均会是 50"
    );
    assert!(
        http_with_suffix(&h, "/posts/create").is_empty(),
        "只完成一张不得发 posts/create"
    );
}

/// 全部完成后恰好一次 posts/create，且最终投影零本地路径 / 零预签名凭据。
#[test]
fn all_completions_emit_single_posts_create_with_full_progress() {
    let mut h = Harness::new(&[1]);
    let prepares = start_two_image_send(&mut h, "tmp-mv3-g01c-004");
    settle_target(
        &mut h,
        prepares[0].0,
        "file-image-001",
        "image/png",
        1_700_000_000_010,
    );
    assert!(http_with_suffix(&h, "/posts/create").is_empty());
    settle_target(
        &mut h,
        prepares[1].0,
        "file-image-002",
        "image/jpeg",
        1_700_000_000_020,
    );

    let posts = http_with_suffix(&h, "/posts/create");
    assert_eq!(posts.len(), 1, "多图必须合并成一条消息、一次 posts/create");
    let body = &posts[0].1;
    assert_eq!(body["type"], "IMAGE");
    assert_eq!(body["props"]["uploadProgress"], 100);
    for (index, file_id) in ["file-image-001", "file-image-002"].iter().enumerate() {
        let file = &body["props"]["files"][index];
        assert_eq!(file["id"], *file_id);
        assert_eq!(file["bucket"], "picture");
        assert_eq!(file["uploadStatus"], "completed");
        assert_eq!(file["uploadProgress"], 100);
        for forbidden in ["mediaHandle", "mediaInput", "localPath", "url", "headers"] {
            assert!(
                file.get(forbidden).is_none(),
                "最终 files[{index}] 不得携带 {forbidden}"
            );
        }
    }
    let rendered = body.to_string();
    for secret in [PATH_A, PATH_B, "signature=short-lived"] {
        assert!(!rendered.contains(secret), "posts/create 泄漏 {secret}");
    }
}

/// 失败断言：第二张图上传失败 ⇒ 整条消息零 posts/create；
/// 失败只落在失败目标上，已完成目标的绝对进度不回退。
#[test]
fn second_image_failure_blocks_posts_create_and_isolates_failed_target() {
    let mut h = Harness::new(&[1]);
    let prepares = start_two_image_send(&mut h, "tmp-mv3-g01c-005");
    settle_target(
        &mut h,
        prepares[0].0,
        "file-image-001",
        "image/png",
        1_700_000_000_010,
    );
    reply(
        &mut h,
        prepares[1].0,
        prepare_outcome("file-image-002", "image/jpeg"),
        1_700_000_000_020,
    );
    let put_stage = stage_corrs(&h, "put");
    h.step(port_reply_ok(put_stage[0]), 1_700_000_000_021);
    let upload = upload_corrs(&h)[0];
    h.step(port_reply_err(upload), 1_700_000_000_022);

    assert!(
        http_with_suffix(&h, "/posts/create").is_empty(),
        "任一目标失败必须阻断整条消息出站"
    );
    let props = persisted_props(&h);
    assert_eq!(props["files"][1]["uploadStatus"], "failed");
    assert_eq!(
        props["files"][0]["uploadStatus"], "completed",
        "失败目标不得污染已完成目标"
    );
    assert_eq!(props["files"][0]["uploadProgress"], 100);
    assert_eq!(
        props["files"][0]["id"], "file-image-001",
        "已完成目标的服务端引用必须保留，重试只补失败目标"
    );
}

/// 失败后立刻重发：上传仍在链路内（未进 UnSend 终态）时，`im_retry_send` 不得并发再发一轮
/// prepare / UploadFile，也不得抢跑 posts/create（幂等：同一时刻最多一个 inflight）。
#[test]
fn retry_while_upload_chain_is_live_does_not_duplicate_outbound() {
    let mut h = Harness::new(&[1]);
    let prepares = start_two_image_send(&mut h, "tmp-mv3-g01c-006");
    settle_target(
        &mut h,
        prepares[0].0,
        "file-image-001",
        "image/png",
        1_700_000_000_010,
    );

    let effects = h.step(
        common::command_json(
            "im_retry_send",
            json!({ "message_id": "tmp-mv3-g01c-006", "req_id": "retry-1" }),
        ),
        1_700_000_000_030,
    );

    assert!(
        http_with_suffix(&h, "/oss/getPresignedUrl").is_empty(),
        "在途上传不得被重发命令再拉一轮 prepare"
    );
    assert!(
        upload_corrs(&h).is_empty(),
        "在途上传不得被重发命令再拉一轮 UploadFile：{effects:?}"
    );
    assert!(
        http_with_suffix(&h, "/posts/create").is_empty(),
        "重发命令不得抢在剩余目标完成前发 posts/create"
    );
}
