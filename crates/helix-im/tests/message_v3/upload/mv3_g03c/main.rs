use helix_core::effect::Effect;
use serde_json::{json, Value};

#[path = "../../../common.rs"]
mod common;
#[path = "../../support/phase1_reference.rs"]
mod phase1_reference;

use common::{port_reply_ok, send_message_cmd_json, EffectKind, Harness};

/// 返回首个 Persist correlation。
fn persist_correlation(effects: &[EffectKind]) -> u64 {
    effects
        .iter()
        .find_map(|effect| match effect {
            EffectKind::Persist(correlation) => Some(*correlation),
            _ => None,
        })
        .unwrap_or_else(|| panic!("missing Persist correlation in {effects:?}"))
}

/// 读取实际 posts/create effect 的结构化 JSON body。
fn posts_create_body(effects: &[Effect]) -> Value {
    effects
        .iter()
        .find_map(|effect| match effect {
            Effect::Http { req, .. } if req.url.ends_with("/posts/create") => req
                .body
                .as_ref()
                .and_then(|body| serde_json::from_slice(body.as_ref()).ok()),
            _ => None,
        })
        .expect("missing posts/create body")
}

/// 证明 MV3-G03c 的 visual 目标参考包完整且未伪装为运行时证据。
#[test]
fn reference_package_is_complete() {
    phase1_reference::assert_reference_gate("MV3-G03c", "upload/mv3-g03c");
}

/// 已有稳定对象引用直接进入 posts/create，不产生 prepare 或 UploadFile effect。
#[test]
fn stable_image_object_reference_skips_the_upload_pipeline() {
    let mut harness = Harness::new(&[1]);
    let initial = harness.step(
        send_message_cmd_json(json!({
            "channel_id": common::cid(1),
            "type": "IMAGE",
            "text": "转发图片",
            "props": {
                "imagePosition": "bottom",
                "files": [{
                    "bucket": "message",
                    "id": "file-existing-001",
                    "uri": "/message/existing.png",
                    "name": "existing.png",
                    "contentType": "image/png",
                    "size": 1024,
                    "width": 640,
                    "height": 480
                }]
            },
            "viewers": ["all"],
            "mentions": []
        })),
        1_700_000_000_000,
    );
    let started = harness.step(
        port_reply_ok(persist_correlation(&initial)),
        1_700_000_000_001,
    );
    let body = posts_create_body(harness.sink.as_slice());

    assert_eq!(
        started
            .iter()
            .filter(|effect| matches!(effect, EffectKind::Http(_)))
            .count(),
        1,
        "only posts/create may be emitted"
    );
    assert!(!started
        .iter()
        .any(|effect| matches!(effect, EffectKind::UploadFile(_))));
    assert_eq!(body["props"]["files"][0]["bucket"], "message");
    assert_eq!(body["props"]["files"][0]["id"], "file-existing-001");
    assert_eq!(body["props"]["files"][0]["uri"], "/message/existing.png");
    assert!(body["props"]["files"][0].get("mediaInput").is_none());
}

/// HTTP(S) URL 或临时上传凭据不得伪装成稳定对象引用绕过上传。
#[test]
fn transient_or_signed_reference_is_rejected_before_any_effect() {
    for file in [
        json!({
            "bucket": "message",
            "id": "file-existing-001",
            "uri": "https://signed.example/existing.png"
        }),
        json!({
            "bucket": "message",
            "id": "file-existing-001",
            "uri": "/message/existing.png",
            "headers": {"Authorization": "secret"}
        }),
    ] {
        let mut harness = Harness::new(&[1]);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            harness.step(
                send_message_cmd_json(json!({
                    "channel_id": common::cid(1),
                    "type": "IMAGE",
                    "text": "非法引用",
                    "props": {
                        "imagePosition": "bottom",
                        "files": [file]
                    }
                })),
                1_700_000_000_000,
            )
        }));
        assert!(result.is_err(), "invalid stable reference must fail closed");
        assert!(harness.sink.is_empty());
    }
}
