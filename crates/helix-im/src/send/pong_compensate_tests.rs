//! UC-4.4 pong gap 补偿消费侧单测（Round-3 偏差修复证伪锚）。
//!
//! 可证伪（HX-C011）：
//! - parse_gap_channels 漏去重 / 取错键（channel_id snake 而非 channelId camel）→ 计数 FAIL
//! - compensate_from_pong 不补 gap channel → Http 计数 0 FAIL
//! - 节流失效（同 channel 5s 内补两次）→ 第二次 Http 非 0 FAIL
//! - hashMismatch 无显式 gap 却触发全量根群补偿 → Http 计数 FAIL
//! - 根群补偿非确定性有序（HashMap 随机序）→ 多跑序断言 FAIL

use super::*;
use crate::channel::Channel;
use crate::state::{test_channel_id, ImState};
use helix_core::effect::Effect;
use helix_core::{Correlation, EffectSink};
use serde_json::json;

fn mk_corr() -> impl FnMut() -> Correlation {
    let mut n = 1u64;
    move || {
        let c = Correlation::from_raw(n);
        n += 1;
        c
    }
}

/// 数 sink 里 sync/notify 的 Http Effect 条数（= 实际触发的补偿 sync 数）。
fn count_sync_http(out: &EffectSink) -> usize {
    out.as_slice()
        .iter()
        .filter(
            |e| matches!(e, Effect::Http { req, .. } if req.url.ends_with("/channel/sync/notify")),
        )
        .count()
}

#[test]
fn parse_gap_channels_dedups_and_reads_camel_channel_id() {
    let a = test_channel_id(1).as_str().to_string();
    let b = test_channel_id(2).as_str().to_string();
    let data = json!({
        "gaps": [
            { "channelId": a, "fromSeq": 1, "maxSeq": 9 },
            { "channelId": b, "fromSeq": 2, "maxSeq": 8 },
            { "channelId": a, "fromSeq": 3 }, // 重复 → 去重
            { "channelId": "not-26-chars" },  // 非法 → 跳
            { "fromSeq": 5 }                  // 缺 channelId → 跳
        ]
    });
    let got = parse_gap_channels(&data);
    assert_eq!(got.len(), 2, "去重 + 非法剔除后应剩 2 个");
    assert_eq!(got[0], test_channel_id(1));
    assert_eq!(got[1], test_channel_id(2));
}

#[test]
fn parse_gap_channels_snake_key_is_not_read() {
    // 证伪：现网 wire 是 camelCase channelId；若实现误读 snake channel_id → 此处取不到 → 空。
    let id = test_channel_id(1).as_str().to_string();
    let data = json!({ "gaps": [ { "channel_id": id } ] });
    assert!(
        parse_gap_channels(&data).is_empty(),
        "snake channel_id 不是契约键，必须取不到"
    );
}

#[test]
fn parse_hash_mismatch_zero_trust_defaults_false() {
    assert!(parse_hash_mismatch(&json!({ "hashMismatch": true })));
    assert!(!parse_hash_mismatch(&json!({ "hashMismatch": false })));
    assert!(!parse_hash_mismatch(&json!({})), "缺字段 → false");
    assert!(
        !parse_hash_mismatch(&json!({ "hashMismatch": "true" })),
        "非 bool → false（零信任）"
    );
}

#[test]
fn gap_channels_trigger_compensation_sync() {
    let mut state = ImState::new();
    let c1 = test_channel_id(1);
    let c2 = test_channel_id(2);
    state.channels.insert(c1, Channel::new(c1, 5));
    state.channels.insert(c2, Channel::new(c2, 7));

    let mut out = EffectSink::new();
    let mut alloc = mk_corr();
    compensate_from_pong(
        &mut state,
        "http://t",
        1_000,
        &[c1, c2],
        false,
        &mut alloc,
        &mut out,
    );

    assert_eq!(
        count_sync_http(&out),
        1,
        "两个 gap channel 合成一次有界批量补偿"
    );
    let cursors = out.as_slice().iter().find_map(|e| match e {
        helix_core::Effect::Http { req, .. } => Some(serde_json::from_slice::<serde_json::Value>(req.body.as_ref().unwrap()).unwrap()),
        _ => None,
    }).unwrap();
    assert_eq!(cursors["cursors"], json!([{"channelId":c1.as_str(),"fromSeq":5},{"channelId":c2.as_str(),"fromSeq":7}]));
}

#[test]
fn explicit_pong_gap_is_prioritized_while_recovery_is_collecting() {
    let mut state = ImState::new();
    let c1 = test_channel_id(1);
    state.channels.insert(c1, Channel::new(c1, 5));
    state.recovery_session.begin("actor");

    let mut out = EffectSink::new();
    let mut alloc = mk_corr();
    compensate_from_pong(
        &mut state,
        "http://t",
        1_000,
        &[c1],
        true,
        &mut alloc,
        &mut out,
    );

    assert_eq!(
        count_sync_http(&out),
        1,
        "明确 gap 必须越过 routine recovery backlog 及时补偿"
    );

    state.channels.get_mut(&c1).unwrap().inflight_sync = None;
    state.sync_scheduler.release_window();
    state.recovery_session.mark_completion_published();
    compensate_from_pong(
        &mut state,
        "http://t",
        1_001,
        &[c1],
        false,
        &mut alloc,
        &mut out,
    );
    assert_eq!(
        count_sync_http(&out),
        1,
        "5s 节流仍阻止同一 channel 立即重复补偿"
    );
}

#[test]
fn hash_only_mismatch_is_deferred_while_recovery_is_collecting() {
    let mut state = ImState::new();
    let c1 = test_channel_id(1);
    state.channels.insert(c1, Channel::new(c1, 5));
    state.recovery_session.begin("actor");

    let mut out = EffectSink::new();
    let mut alloc = mk_corr();
    compensate_from_pong(
        &mut state,
        "http://t",
        1_000,
        &[],
        true,
        &mut alloc,
        &mut out,
    );

    assert_eq!(
        count_sync_http(&out),
        0,
        "无明确 channel 的 hash mismatch 不得续填 recovery 队列"
    );
}

#[test]
fn unknown_gap_channel_is_skipped_not_panicked() {
    // gap 指向不存在的 channel（已注销 / 从未注册）→ enqueue_and_drain 内部 None => skip，0 补偿。
    let mut state = ImState::new();
    let missing = test_channel_id(9);
    let mut out = EffectSink::new();
    let mut alloc = mk_corr();
    compensate_from_pong(
        &mut state,
        "http://t",
        1_000,
        &[missing],
        false,
        &mut alloc,
        &mut out,
    );
    assert_eq!(
        count_sync_http(&out),
        0,
        "不存在的 channel 不补偿（不 panic）"
    );
}

#[test]
fn projection_terminal_gap_channel_is_not_compensated() {
    let mut state = ImState::new();
    let closed = test_channel_id(10);
    let mut channel = Channel::new(closed, 0);
    channel.mark_projection_terminal(crate::state::Seq(0));
    state.channels.insert(closed, channel);

    let mut out = EffectSink::new();
    let mut alloc = mk_corr();
    compensate_from_pong(
        &mut state,
        "http://t",
        1_000,
        &[closed],
        true,
        &mut alloc,
        &mut out,
    );

    assert_eq!(
        count_sync_http(&out),
        0,
        "deleted/closed channel must not be reintroduced by PongGap"
    );
    assert_eq!(state.sync_scheduler.pending_len(), 0);
}

#[test]
fn throttle_dedups_same_channel_within_5s() {
    let mut state = ImState::new();
    let c1 = test_channel_id(1);
    state.channels.insert(c1, Channel::new(c1, 5));

    // 第一次：补偿（占在途窗口）。
    let mut out1 = EffectSink::new();
    let mut alloc = mk_corr();
    compensate_from_pong(
        &mut state,
        "http://t",
        1_000,
        &[c1],
        false,
        &mut alloc,
        &mut out1,
    );
    assert_eq!(count_sync_http(&out1), 1, "首次补偿");

    // 模拟首次 sync 回报释放在途窗口（否则下次被 per-channel inflight 守卫挡，掩盖节流验证）。
    state.channels.get_mut(&c1).unwrap().inflight_sync = None;
    state.sync_scheduler.release_window();

    // 4s 后（< 5s 节流）：should_pong_compensate 返回 false → 0 补偿。
    let mut out2 = EffectSink::new();
    compensate_from_pong(
        &mut state,
        "http://t",
        5_000,
        &[c1],
        false,
        &mut alloc,
        &mut out2,
    );
    assert_eq!(count_sync_http(&out2), 0, "5s 内同 channel 节流跳过");

    // 6s 后（>= 5s）：闸门重开 → 补偿。
    let mut out3 = EffectSink::new();
    compensate_from_pong(
        &mut state,
        "http://t",
        6_001,
        &[c1],
        false,
        &mut alloc,
        &mut out3,
    );
    assert_eq!(count_sync_http(&out3), 1, "超 5s 间隔后允许再补偿");
}

#[test]
fn hash_mismatch_without_explicit_gaps_does_not_fan_out() {
    let mut state = ImState::new();
    let c1 = test_channel_id(1);
    let c2 = test_channel_id(2);
    let c3 = test_channel_id(3);
    state.channels.insert(c1, Channel::new(c1, 1));
    state.channels.insert(c2, Channel::new(c2, 1));
    state.channels.insert(c3, Channel::new(c3, 1));

    let mut out = EffectSink::new();
    let mut alloc = mk_corr();
    // 无 gap，hashMismatch=true 只说明根集合不一致；逐根 sync 不能修复集合差异。
    compensate_from_pong(
        &mut state,
        "http://t",
        1_000,
        &[],
        true,
        &mut alloc,
        &mut out,
    );
    assert_eq!(
        count_sync_http(&out),
        0,
        "hashMismatch 不得放大成全量根群周期补偿"
    );
}

#[test]
fn gap_remains_bounded_when_hash_mismatch_is_also_true() {
    // gap 命中 c1，同时 hashMismatch → 只补显式 c1，不能附带扫描 c2。
    let mut state = ImState::new();
    let c1 = test_channel_id(1);
    let c2 = test_channel_id(2);
    state.channels.insert(c1, Channel::new(c1, 1));
    state.channels.insert(c2, Channel::new(c2, 1));

    let mut out = EffectSink::new();
    let mut alloc = mk_corr();
    compensate_from_pong(
        &mut state,
        "http://t",
        1_000,
        &[c1],
        true,
        &mut alloc,
        &mut out,
    );
    assert_eq!(
        count_sync_http(&out),
        1,
        "hashMismatch 同时存在时仍只补显式 gap channel"
    );
}
