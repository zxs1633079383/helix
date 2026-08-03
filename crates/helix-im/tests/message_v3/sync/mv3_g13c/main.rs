//! MV3-G13c：Helix 自驱同步的连续性、预算和 TooLong 原子边界。
//!
//! 测试只驱动真实 `ImModule::handle` 与 PortReply owner；不会调用同步 helper 旁路生产链。

#[path = "../../../common.rs"]
mod common;

use common::{
    channel_id, command_json, enveloped, inbound_event, port_reply_err, port_reply_ok,
    port_reply_with_body, sync_reply_events, sync_reply_too_long, EffectKind, Harness,
};
use helix_core::effect::{Effect, StorageOp};

/// 从当前 effect batch 取唯一 HTTP correlation。
fn http_corr(h: &Harness) -> u64 {
    h.sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Http { corr, .. } => Some(corr.raw()),
            _ => None,
        })
        .expect("sync path must produce an HTTP correlation")
}

/// 从当前 effect batch 取唯一 PersistAtomic correlation。
fn atomic_corr(h: &Harness) -> u64 {
    h.sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::PersistAtomic { corr, .. } => Some(corr.raw()),
            _ => None,
        })
        .expect("sync path must produce an atomic persistence correlation")
}

/// 判断批次是否包含会降低/重置游标的 fire-and-forget 写。
fn has_cursor_fire(h: &Harness) -> bool {
    h.sink.as_slice().iter().any(|effect| {
        matches!(effect, Effect::PersistFire { ops } if ops.iter().any(|op| {
            matches!(op, StorageOp::MonotonicUpsert(spec) if spec.table == "channel_event_cursor")
        }))
    })
}

/// G13c 的单频道游标必须在持久化回执后才推进，重复帧不产生第二条写路径。
#[test]
fn duplicate_and_out_of_order_events_are_idempotent_per_channel() {
    let mut h = Harness::new_with_identity(&[1, 2], "viewer-g13c", "company-g13c", "", "", "");

    // 先制造 ch1 缺口；ch2 不得被同一补偿污染。
    let initial = h.step(inbound_event(1, 6, 1), 1_000);
    assert!(initial
        .iter()
        .any(|kind| matches!(kind, EffectKind::Http(_))));
    assert_eq!(h.module.cursor_for(channel_id(2)), Some(0));
    let sync = http_corr(&h);

    // 权威事件按 seq 排序后一次原子提交；回执前 cursor 仍停在旧值。
    let effects = h.step(
        port_reply_with_body(sync, enveloped(sync_reply_events(1, &[1, 2, 3, 4]))),
        2_000,
    );
    let persist = effects
        .iter()
        .find_map(|kind| match kind {
            EffectKind::PersistAtomic(corr) => Some(*corr),
            _ => None,
        })
        .expect("ordered sync batch must use PersistAtomic");
    assert_eq!(h.module.cursor_for(channel_id(1)), Some(0));
    h.step(port_reply_ok(persist), 2_100);
    assert_eq!(h.module.cursor_for(channel_id(1)), Some(4));

    // seq=5 到达时应填平前序缺口，并让先到的 seq=6 一并放行。
    let gap = h.step(inbound_event(1, 5, 1), 3_000);
    let gap_persist = gap
        .iter()
        .find_map(|kind| match kind {
            EffectKind::PersistAtomic(corr) => Some(*corr),
            _ => None,
        })
        .expect("ordered gate event must wait for its atomic receipt");
    let flushed = h.step(port_reply_ok(gap_persist), 3_100);
    if let Some(next_persist) = flushed.iter().find_map(|kind| match kind {
        EffectKind::PersistAtomic(corr) => Some(*corr),
        _ => None,
    }) {
        h.step(port_reply_ok(next_persist), 3_200);
    }
    assert_eq!(
        h.module.cursor_for(channel_id(1)),
        Some(6),
        "seq=5 should release the pre-buffered seq=6"
    );

    // 重复 seq=6 只能 no-op；不能产生第二条 cursor 写或业务终态。
    let duplicate = h.step(inbound_event(1, 6, 1), 3_300);
    assert!(!duplicate
        .iter()
        .any(|kind| matches!(kind, EffectKind::PersistFire)));
    assert!(!has_cursor_fire(&h));
    assert_eq!(h.module.cursor_for(channel_id(2)), Some(0));
}

/// G13c 单批上限是 500；满批回执后才续拉，空/短批不触发无界自驱循环。
#[test]
fn five_hundred_batch_continues_only_after_atomic_commit() {
    let mut h = Harness::new_with_identity(&[1], "viewer-g13c", "company-g13c", "", "", "");
    let _ = h.step(inbound_event(1, 5, 1), 1_000);
    let sync = http_corr(&h);

    let first = h.step(
        port_reply_with_body(
            sync,
            enveloped(sync_reply_events(1, &(1..=500).collect::<Vec<_>>())),
        ),
        2_000,
    );
    let persist = first
        .iter()
        .find_map(|kind| match kind {
            EffectKind::PersistAtomic(corr) => Some(*corr),
            _ => None,
        })
        .expect("500 event batch must be atomic");
    assert_eq!(h.module.cursor_for(channel_id(1)), Some(0));

    let continuation = h.step(port_reply_ok(persist), 2_100);
    assert_eq!(h.module.cursor_for(channel_id(1)), Some(500));
    assert_eq!(
        continuation
            .iter()
            .filter(|kind| matches!(kind, EffectKind::Http(_)))
            .count(),
        1,
        "continuation must be emitted only after cursor commit"
    );
    let next_sync = http_corr(&h);
    let short = h.step(
        port_reply_with_body(next_sync, enveloped(sync_reply_events(1, &[501]))),
        2_200,
    );
    let short_persist = short
        .iter()
        .find_map(|kind| match kind {
            EffectKind::PersistAtomic(corr) => Some(*corr),
            _ => None,
        })
        .expect("short batch must still commit atomically");
    let done = h.step(port_reply_ok(short_persist), 2_300);
    assert_eq!(h.module.cursor_for(channel_id(1)), Some(501));
    assert!(!done.iter().any(|kind| matches!(kind, EffectKind::Http(_))));
}

/// 5000 是一次回放预算，不是 retention 下限；预算内批次仍走普通 ordered atomic commit。
#[test]
fn five_thousand_replay_budget_is_not_retention_reset() {
    let mut h = Harness::new_with_identity(&[1], "viewer-g13c", "company-g13c", "", "", "");
    let _ = h.step(inbound_event(1, 5_000, 1), 1_000);
    let sync = http_corr(&h);
    let reply = sync_reply_events(1, &(1..=5_000).collect::<Vec<_>>());
    let effects = h.step(port_reply_with_body(sync, enveloped(reply)), 2_000);
    let persist = atomic_corr(&h);
    assert!(effects
        .iter()
        .any(|kind| matches!(kind, EffectKind::PersistAtomic(_))));
    assert!(!effects
        .iter()
        .any(|kind| matches!(kind, EffectKind::Http(_))));
    assert_eq!(h.module.cursor_for(channel_id(1)), Some(0));
    h.step(port_reply_ok(persist), 2_100);
    assert_eq!(h.module.cursor_for(channel_id(1)), Some(5_000));
}

/// TooLong 失败时，窗口与 channel_event_cursor 必须保持旧值；恢复不得先 reset。
#[test]
fn too_long_failure_preserves_old_window_and_cursor() {
    let mut h = Harness::new_with_identity(&[1], "viewer-g13c", "company-g13c", "", "", "");
    let _ = h.step(inbound_event(1, 5, 1), 1_000);
    let sync = http_corr(&h);
    h.step(
        port_reply_with_body(sync, enveloped(sync_reply_too_long(1, 100))),
        2_000,
    );

    // TooLong 仅启动单频道 authoritative reload；旧 cursor/window 在成功前不可改变。
    assert_eq!(h.module.cursor_for(channel_id(1)), Some(0));
    assert!(!has_cursor_fire(&h));
    let reload = http_corr(&h);
    h.step(port_reply_err(reload), 2_100);
    assert_eq!(h.module.cursor_for(channel_id(1)), Some(0));
    assert!(!has_cursor_fire(&h));
}

/// Snapshot/TooLong 的成功路径必须以同一 PersistAtomic 同批提交 window 与 cursor，
/// 且 late generation 的旧持久化回执不得重新推进游标。
#[test]
fn too_long_success_is_atomic_and_late_reply_is_ignored() {
    let mut h = Harness::new_with_identity(&[1], "viewer-g13c", "company-g13c", "", "", "");
    let _ = h.step(inbound_event(1, 5, 1), 1_000);
    let sync = http_corr(&h);
    h.step(
        port_reply_with_body(sync, enveloped(sync_reply_too_long(1, 100))),
        2_000,
    );
    let reload = http_corr(&h);

    // 首屏回包使用一条带 eventSeq 的权威 post，storage owner 负责同批窗口+cursor。
    let body = serde_json::json!({
        "status": "SUCCESS",
        "data": { "posts": [{
            "id": "g13c-post-100",
            "channelId": common::cid(1),
            "eventSeq": 100,
            "userId": "author",
            "type": "TEXT",
            "message": "authoritative",
            "createAt": 1700000100_i64,
            "viewers": ["viewer-g13c"]
        }] }
    });
    let raw = bytes::Bytes::from(serde_json::to_vec(&body).expect("reload fixture serializes"));
    let effects = h.step(port_reply_with_body(reload, common::enveloped(raw)), 2_100);
    let persist = atomic_corr(&h);
    assert!(effects
        .iter()
        .any(|kind| matches!(kind, EffectKind::PersistAtomic(_))));
    let atomic_ops = h
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::PersistAtomic { corr, ops } if corr.raw() == persist => Some(ops),
            _ => None,
        })
        .expect("TooLong reload must retain its atomic operation set");
    assert!(atomic_ops.iter().any(|op| {
        matches!(op, StorageOp::MonotonicUpsert(spec) if spec.table == "channel_event_cursor")
    }));
    assert_eq!(h.module.cursor_for(channel_id(1)), Some(0));

    // 成功回执才替换窗口并推进 cursor；重复迟到回执因 corr 单次消费而不得二次推进。
    h.step(port_reply_ok(persist), 2_200);
    assert_eq!(h.module.cursor_for(channel_id(1)), Some(100));
    h.step(port_reply_ok(persist), 2_300);
    assert_eq!(h.module.cursor_for(channel_id(1)), Some(100));
}

/// 身份切换推进 runtime generation 后，旧 TooLong HTTP/持久化回包必须 fail-closed。
#[test]
fn too_long_generation_is_dropped_on_identity_reset() {
    let mut h = Harness::new_with_identity(&[1], "viewer-a", "company-a", "", "", "");
    let _ = h.step(inbound_event(1, 5, 1), 1_000);
    let sync = http_corr(&h);
    h.step(
        port_reply_with_body(sync, enveloped(sync_reply_too_long(1, 100))),
        2_000,
    );
    let reload = http_corr(&h);
    let body = serde_json::json!({
        "status": "SUCCESS",
        "data": { "posts": [{
            "id": "g13c-generation-post",
            "channelId": common::cid(1),
            "eventSeq": 100,
            "userId": "author",
            "type": "TEXT",
            "message": "authoritative",
            "createAt": 1700000100_i64
        }] }
    });
    h.step(
        port_reply_with_body(
            reload,
            enveloped(bytes::Bytes::from(
                serde_json::to_vec(&body).expect("generation fixture serializes"),
            )),
        ),
        2_100,
    );
    let persist = atomic_corr(&h);

    h.step(
        command_json(
            "__helix_runtime_identity",
            serde_json::json!({
                "auth_user_id": "viewer-b",
                "company_id": "company-b"
            }),
        ),
        2_200,
    );
    assert_eq!(h.module.cursor_for(channel_id(1)), None);
    let late = h.step(port_reply_ok(persist), 2_300);
    assert!(late.is_empty(), "old generation reply must be ignored");
    assert_eq!(h.module.cursor_for(channel_id(1)), None);
}
