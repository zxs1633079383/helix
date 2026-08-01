use serde_json::json;

#[path = "../../../common.rs"]
mod common;
#[path = "../../support/phase1_reference.rs"]
mod phase1_reference;

use common::{send_message_cmd_json, EffectKind, Harness};

/// 构造指定声明大小的 FILE 发送命令。
fn file_send(size: u64) -> helix_core::Tick {
    send_message_cmd_json(json!({
        "channel_id": common::cid(1),
        "type": "FILE",
        "text": "",
        "props": {
            "file": {
                "mediaInput": {
                    "localPath": "/host-private/staged/opaque-handle",
                    "fileName": "boundary.zip",
                    "contentType": "application/zip",
                    "size": size,
                    "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                }
            }
        },
        "viewers": ["all"],
        "mentions": []
    }))
}

/// 证明 MV3-G02d 的 visual 目标参考包完整且未伪装为运行时证据。
#[test]
fn reference_package_is_complete() {
    phase1_reference::assert_reference_gate("MV3-G02d", "upload/mv3-g02d");
}

/// 64 MiB 边界值可进入唯一 optimistic Persist，且未提前上传或发送 HTTP。
#[test]
fn exact_64_mib_is_accepted_by_the_helix_media_boundary() {
    let mut harness = Harness::new(&[1]);
    let effects = harness.step(file_send(64 << 20), 1_700_000_000_000);

    assert_eq!(
        effects
            .iter()
            .filter(|effect| matches!(effect, EffectKind::PersistAtomic(_)))
            .count(),
        1,
        "unexpected exact-boundary effects: {effects:?}"
    );
    assert!(!effects.iter().any(|effect| matches!(
        effect,
        EffectKind::Http(_)
            | EffectKind::UploadFile(_)
            | EffectKind::Emit
            | EffectKind::ScheduleTimer(_)
    )));
}

/// 64 MiB+1 在 Helix 规划前失败，保持 Persist/HTTP/Upload/Event/Timer 五项为零。
#[test]
fn over_64_mib_is_rejected_with_five_zero_side_effect_classes() {
    let mut harness = Harness::new(&[1]);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        harness.step(file_send((64 << 20) + 1), 1_700_000_000_000)
    }));

    assert!(result.is_err(), "oversize media command must fail closed");
    assert!(harness.sink.is_empty());
    assert_eq!(harness.module.pending_send_count(), 0);
    assert!(harness.effect_log.is_empty());
}
