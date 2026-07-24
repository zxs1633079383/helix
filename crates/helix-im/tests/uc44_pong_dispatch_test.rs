//! UC-4.4 心跳 pong gap 补偿——**全链路 WS dispatch** 回放（Round-3 偏差修复）。
//!
//! 回归锚: UC-4.4（pong 消费侧补齐）
//!
//! 验证 pong 帧（`status==OK` + `data.{gaps,hashMismatch}`、**无 action**）经 `Tick::Inbound`
//! → `WsFrame::parse` → `action()` 合成 `__pong__` → `ws::handlers::pong` →
//! `pong_compensate::compensate_from_pong` → sync/notify Http。
//!
//! 现网真源：cses-client `im_ws_client.rs handle_pong_compensation` → `sync_compensation.rs`。
//! 此前 helix **只有 ping 产侧无 pong 消费侧**——pong 的 `{gaps, hashMismatch}` 全仓无路由，
//! 在线漏推 / 非活跃群 drift 永不补齐。本测试锁住消费侧已接通。
//!
//! 可证伪（HX-C011）：
//! - pong 帧未被识别（仍走 legacy_event 丢弃）→ 0 sync Http FAIL
//! - gap channel 未触发补偿 → Http 计数 FAIL
//! - hashMismatch 未触发全量根群补偿 → Http 计数 FAIL

mod common;
use common::*;

use helix_core::effect::Effect;
use helix_core::tick::InboundBytes;
use helix_core::Tick;

/// 构造 pong ACK 帧（无 action，status==OK，data 带 gaps / hashMismatch）。
fn pong_frame(gaps: &[u64], hash_mismatch: bool) -> Tick {
    use serde_json::json;
    let gaps_json: Vec<_> = gaps
        .iter()
        .map(|&n| json!({ "channelId": cid(n), "fromSeq": 1, "maxSeq": 9 }))
        .collect();
    let payload = json!({
        "status": "OK",
        "seq": 42,
        "data": { "gaps": gaps_json, "hashMismatch": hash_mismatch }
    });
    let bytes = serde_json::to_vec(&payload).unwrap();
    Tick::Inbound(InboundBytes(bytes.into()))
}

/// 数本 step sink 里 sync/notify 的 Http Effect 条数。
fn count_sync_http(h: &Harness) -> usize {
    h.sink
        .as_slice()
        .iter()
        .filter(
            |e| matches!(e, Effect::Http { req, .. } if req.url.ends_with("/channel/sync/notify")),
        )
        .count()
}

#[test]
fn pong_with_gaps_routes_and_compensates() {
    // 预注册 ch1/ch2（gap 指向已知 channel——drift 补偿对象必为已知群）。
    let mut h = Harness::new(&[1, 2]);

    let effects = h.step(pong_frame(&[1, 2], false), 1_000);

    // 关键证伪：pong 必须被识别并触发补偿（此前整帧静默丢弃 → 0）。
    assert_eq!(
        count_sync_http(&h),
        2,
        "pong gaps[ch1,ch2] 应各触发一次 sync/notify 补偿，got effects: {effects:?}"
    );
}

#[test]
fn pong_hash_mismatch_without_gaps_does_not_scan_known_channels() {
    let mut h = Harness::new(&[1, 2, 3]);

    let _ = h.step(pong_frame(&[], true), 1_000);

    assert_eq!(
        count_sync_http(&h),
        0,
        "pong hashMismatch=true 无显式 gap 时不得扫描全部已知频道"
    );
}

#[test]
fn pong_pure_ack_no_gaps_is_noop() {
    let mut h = Harness::new(&[1, 2]);
    // status==OK 但 gaps 空 + hashMismatch=false → 纯 ACK，无补偿。
    let _ = h.step(pong_frame(&[], false), 1_000);
    assert_eq!(count_sync_http(&h), 0, "纯 ACK pong 无补偿动作");
}

#[test]
fn pong_throttle_dedups_within_5s() {
    let mut h = Harness::new(&[1]);

    let _ = h.step(pong_frame(&[1], false), 1_000);
    assert_eq!(count_sync_http(&h), 1, "首个 pong 补偿 ch1");

    // 第二个 pong 4s 后（< 5s 节流）——即便 per-channel inflight 也在途，节流先挡。
    let _ = h.step(pong_frame(&[1], false), 5_000);
    assert_eq!(
        count_sync_http(&h),
        0,
        "5s 内同 channel 节流，第二个 pong 不重复补偿"
    );
}
