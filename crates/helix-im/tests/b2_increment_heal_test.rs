//! B2 验收：increment_end 冷启动自愈（heal_incomplete_backfill_once 对齐）
//!
//! 回归锚: B2 —— increment 收尾对「服务端说已追平(need_sync=false)、但 local cursor 实际
//! 落后服务端 increment 水位」的冷启动卡死 channel 补发 sync。
//!
//! 真源: cses-client `heal_incomplete_backfill_once`（post.rs:666-715）——冷启动
//! 「cursor 推到 maxSeq 但本地只灌首屏 20」的卡死 channel，靠「本地落后水位」识别后补全量回填。
//! helix sans-IO 核等价信号 = local cursor < 服务端 increment 水位（inc.last_event_seq）。
//!
//! ## 验收场景
//!
//! 1. need_sync=false 但 cursor 落后水位 → heal 补发 sync（主 resync 因 skip 不发，靠 heal 补回）。
//! 2. need_sync=false 且 cursor 已追平水位 → 不补发（非卡死，无需 heal）。
//! 3. once-only：同一会话第二次 global-end 不再补发（防 phantom-heavy 死循环）。
//! 4. 新会话(第二次 hello) reset 后允许再 heal 一次。
//! 5. need_sync=true 落后 channel 不被 heal 重复发（主 resync 已发 + inflight 守卫去重）。

mod common;
use common::*;

use helix_core::effect::TransportId;
use helix_core::{Effect, Tick};
use helix_im::state::{ChannelId, Seq};
use helix_im::sync_session::IncrementChannel;

/// 收集本批所有 Http effect 的 (method, url)。
fn http_reqs(sink: &helix_core::EffectSink) -> Vec<(String, String)> {
    sink.as_slice()
        .iter()
        .filter_map(|e| match e {
            Effect::Http { req, .. } => Some((req.method.clone(), req.url.clone())),
            _ => None,
        })
        .collect()
}

/// sync/notify Http 数量（heal / resync 都走它）。
fn sync_http_count(sink: &helix_core::EffectSink) -> usize {
    http_reqs(sink)
        .iter()
        .filter(|(_, url)| url.ends_with("/channel/sync/notify"))
        .count()
}

/// 取首个 sync/notify Http 的 corr（用于回放其 PortReply 释放 inflight 守卫）。
fn first_sync_corr(sink: &helix_core::EffectSink) -> u64 {
    sink.as_slice()
        .iter()
        .find_map(|e| match e {
            Effect::Http { corr, req } if req.url.ends_with("/channel/sync/notify") => {
                Some(corr.raw())
            }
            _ => None,
        })
        .expect("应有 sync/notify Http")
}

fn inc(channel_id: u64, last_event_seq: u64, need_sync: bool) -> IncrementChannel {
    IncrementChannel {
        channel_id: ChannelId::from_test_u64(channel_id),
        last_event_seq: Seq(last_event_seq),
        need_sync,
        raw: bytes::Bytes::from("{}"),
    }
}

/// 起一个完成 hello 握手的 Harness（清干净 sink）。
fn connected_harness(preregistered: &[u64]) -> Harness {
    let mut h = Harness::new(preregistered);
    h.start(0);
    h.step(Tick::Connected(TransportId::from_raw(0)), 100);
    h.step(hello_inbound("CONN_HEAL"), 200);
    h.sink.clear();
    h
}

// ─── ① need_sync=false 但 cursor 落后水位 → heal 补发 sync ──────────────────────

#[test]
fn heal_resyncs_skipped_channel_lagging_behind_increment_watermark() {
    // 预注册 channel=7 cursor=20（冷启动只灌了首屏 20）。
    let mut h = connected_harness(&[]);
    h.module.register_channel(ChannelId::from_test_u64(7), 20);

    // increment 帧：need_sync=false（服务端"已追平"），但水位=999 远超本地 cursor=20 → 卡死信号。
    h.module.ingest_increment(&inc(7, 999, false), &mut h.sink);
    h.sink.clear();

    // global end：主 resync 因 7 在 skip 集不发；heal 识别 cursor(20) < 水位(999) → 补发。
    h.module.ingest_increment_end(None, &mut h.sink);

    assert_eq!(
        sync_http_count(&h.sink),
        1,
        "need_sync=false 但 cursor 落后水位的卡死 channel 须经 heal 补发一次 sync，got: {:?}",
        http_reqs(&h.sink)
    );
    let (method, url) = &http_reqs(&h.sink)[0];
    assert_eq!(method, "POST", "heal sync 须 POST");
    assert!(
        url.ends_with("/channel/sync/notify"),
        "heal 须走 sync/notify，got: {}",
        url
    );
}

// ─── ② need_sync=false 且 cursor 已追平水位 → 不补发 ──────────────────────────────

#[test]
fn heal_skips_caughtup_channel() {
    // channel=8 cursor=500，水位也是 500（已追平，非卡死）。
    let mut h = connected_harness(&[]);
    h.module.register_channel(ChannelId::from_test_u64(8), 500);

    h.module.ingest_increment(&inc(8, 500, false), &mut h.sink);
    h.sink.clear();

    h.module.ingest_increment_end(None, &mut h.sink);

    assert_eq!(
        sync_http_count(&h.sink),
        0,
        "cursor 已追平水位（非卡死）不应 heal 补发，got: {:?}",
        http_reqs(&h.sink)
    );
}

// ─── ③ once-only：同会话第二次 global-end 不再 heal（防 phantom 死循环）────────────

#[test]
fn heal_runs_once_per_session_batch() {
    // phantom-heavy 模拟：cursor=20 恒 < 水位=999（phantom 不推 cursor，HX-C008 §1.6）。
    let mut h = connected_harness(&[]);
    h.module.register_channel(ChannelId::from_test_u64(9), 20);
    h.module.ingest_increment(&inc(9, 999, false), &mut h.sink);

    // 第一次 global-end → heal 补发一次。
    h.sink.clear();
    h.module.ingest_increment_end(None, &mut h.sink);
    assert_eq!(
        sync_http_count(&h.sink),
        1,
        "首次 global-end 须 heal 补发一次"
    );

    // 第二次 global-end（同会话，cursor 仍 20 落后）→ once-only 守卫拦住，不再补发。
    h.sink.clear();
    h.module.ingest_increment_end(None, &mut h.sink);
    assert_eq!(
        sync_http_count(&h.sink),
        0,
        "同会话第二次 global-end 不得再 heal（once-only 防 phantom 死循环），got: {:?}",
        http_reqs(&h.sink)
    );
}

// ─── ④ 新会话(第二次 hello) reset 后允许再 heal 一次 ─────────────────────────────

#[test]
fn heal_resets_and_runs_again_on_new_session() {
    let mut h = connected_harness(&[]);
    h.module.register_channel(ChannelId::from_test_u64(11), 20);
    h.module.ingest_increment(&inc(11, 999, false), &mut h.sink);
    h.sink.clear();
    h.module.ingest_increment_end(None, &mut h.sink);
    assert_eq!(sync_http_count(&h.sink), 1, "会话1 heal 一次");

    // 回放会话1 heal sync 的 PortReply 释放 inflight 守卫（否则下会话被 B1 inflight 守卫挡掉）。
    // phantom-heavy 模拟：no_change 锚 cursor 到 20（events 尾全 phantom，cursor 推不过 20，
    // HX-C008 §1.6）→ 下会话 cursor(20) 仍 < 水位(999) 维持卡死，验证「reset 后能再 heal」。
    let corr1 = first_sync_corr(&h.sink);
    h.step(
        port_reply_with_body(corr1, sync_reply_no_change(11, 20)),
        250,
    );

    // 第二次 hello（重连）→ reset_increment_batch 清 backfill_healed + increment_target。
    h.step(hello_inbound("CONN_HEAL2"), 300);
    // 重新喂 increment（新会话仍卡死：cursor 20 < 水位 999）。
    h.module.ingest_increment(&inc(11, 999, false), &mut h.sink);
    h.sink.clear();
    h.module.ingest_increment_end(None, &mut h.sink);
    assert_eq!(
        sync_http_count(&h.sink),
        1,
        "新会话 reset 后须允许再 heal 一次，got: {:?}",
        http_reqs(&h.sink)
    );
}

// ─── ⑤ need_sync=true 落后 channel 不被 heal 重复发（inflight 守卫去重）────────────

#[test]
fn heal_does_not_double_send_need_sync_true_channel() {
    // channel=13 need_sync=true cursor=20 < 水位 999：主 resync 已发 1 次。
    let mut h = connected_harness(&[]);
    h.module.register_channel(ChannelId::from_test_u64(13), 20);
    h.module.ingest_increment(&inc(13, 999, true), &mut h.sink);
    h.sink.clear();

    h.module.ingest_increment_end(None, &mut h.sink);

    // 主 resync 发 1 次；heal 的 emit_proactive_resync_for 因 inflight 守卫跳过 → 总计仍 1 次。
    assert_eq!(
        sync_http_count(&h.sink),
        1,
        "need_sync=true 落后 channel 主 resync 已发，heal 不得重复发（inflight 守卫去重），got: {:?}",
        http_reqs(&h.sink)
    );
}
