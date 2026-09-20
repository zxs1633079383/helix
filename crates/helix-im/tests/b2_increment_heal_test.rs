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
use helix_im::state::Seq;
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

/// 取首个 sync/notify Http 的 body 里 cursors[0].fromSeq（断言冷启动 from_seq）。
fn first_sync_from_seq(sink: &helix_core::EffectSink) -> u64 {
    sink.as_slice()
        .iter()
        .find_map(|e| match e {
            Effect::Http { req, .. } if req.url.ends_with("/channel/sync/notify") => {
                let body = req.body.as_ref().expect("sync/notify 须带 body");
                let v: serde_json::Value =
                    serde_json::from_slice(body.as_ref()).expect("sync/notify body 须为合法 JSON");
                v["cursors"][0]["fromSeq"].as_u64()
            }
            _ => None,
        })
        .expect("应有 sync/notify Http 且含 fromSeq")
}

fn inc(channel_num: u64, last_event_seq: u64, need_sync: bool) -> IncrementChannel {
    IncrementChannel {
        channel_id: channel_id(channel_num),
        last_event_seq: Seq(last_event_seq),
        need_sync,
        unread_post_id: None,
        last_read_seq: None,
        projection_revision: None,
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
    h.module.register_channel(channel_id(7), 20);

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
    h.module.register_channel(channel_id(8), 500);

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
    h.module.register_channel(channel_id(9), 20);
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
    h.module.register_channel(channel_id(11), 20);
    h.module.ingest_increment(&inc(11, 999, false), &mut h.sink);
    h.sink.clear();
    h.module.ingest_increment_end(None, &mut h.sink);
    assert_eq!(sync_http_count(&h.sink), 1, "会话1 heal 一次");

    // 回放会话1 heal sync 的 PortReply 释放 inflight 守卫（否则下会话被 B1 inflight 守卫挡掉）。
    // phantom-heavy 模拟：no_change 锚 cursor 到 20（events 尾全 phantom，cursor 推不过 20，
    // HX-C008 §1.6）→ 下会话 cursor(20) 仍 < 水位(999) 维持卡死，验证「reset 后能再 heal」。
    let corr1 = first_sync_corr(&h.sink);
    // dev-zlc：sync 回报走 ADR-007 信封，handle_sync_reply 解信封后才解析 → 回放须先包信封。
    h.step(
        port_reply_with_body(corr1, enveloped(sync_reply_no_change(11, 20))),
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

// ─── ⑥ 冷启动真新 channel：sync from_seq==0（非 lastEventSeq）+ B2 heal 触发 ─────────

/// 回归锚: B2 / 修 todo#1 —— 冷启动**真新** channel（DB 无 cursor，on_start Scan 未载入，
/// 经 or_insert_with 首次注册）的内存 cursor 种子 = 0，**不是**帧的 lastEventSeq。
///
/// 这是 backfill 缺口根因的端到端证伪：
/// - 误种子 lastEventSeq → from_seq=cursor=max → Go no_change → 离线 backfill 永不触发；
///   且 B2 heal 的 cursor<target 永假（cursor==target==lastEventSeq）→ heal 连带废。
/// - 修后 cursor=0 → ① 主 resync 的 from_seq=0（拉全量离线事件）② heal 判 0<water=stuck。
///
/// 与上面 ①-⑤ 不同：这些用例都先 register_channel 显式预种 cursor，绕过了 or_insert_with
/// 冷启动种子路径。本用例**不** register_channel，专测真新 channel 的 or_insert_with 种子。
#[test]
fn cold_start_brand_new_channel_syncs_from_zero_not_watermark() {
    // 不 register_channel：channel=21 是真新（or_insert_with 首次注册）。need_sync=true。
    let mut h = connected_harness(&[]);
    h.module.ingest_increment(&inc(21, 999, true), &mut h.sink);

    // 注册即冷启动，内存 cursor 种子须 0（本地未确认任何 event），不是水位 999。
    assert_eq!(
        h.module.cursor_for(channel_id(21)),
        Some(0),
        "冷启动真新 channel cursor 种子须为 0，不得用 lastEventSeq(999)"
    );

    // global end → 主 resync 对 21 发 sync，from_seq 必须 == 0（拉全量离线事件），非 999。
    h.sink.clear();
    h.module.ingest_increment_end(None, &mut h.sink);
    assert_eq!(
        sync_http_count(&h.sink),
        1,
        "冷启动 channel global-end 须发一次 sync，got: {:?}",
        http_reqs(&h.sink)
    );
    assert_eq!(
        first_sync_from_seq(&h.sink),
        0,
        "冷启动 sync from_seq 必须 == 0（否则 Go no_change，离线 backfill 永不触发），不得 == lastEventSeq(999)"
    );
}

/// 回归锚: B2 / 修 todo#1 —— need_sync=false 的冷启动真新 channel：主 resync 跳过（skip 集），
/// 但 B2 heal 必须识别 cursor(0) < target(999) = stuck → 补发 sync from_seq==0。
/// 修前 cursor==target==999 → heal 永不判 stuck → 离线 backfill 彻底丢失。
#[test]
fn cold_start_skipped_channel_healed_with_from_zero() {
    // 不 register_channel：channel=22 真新；need_sync=false → 进 skip 集（主 resync 不发）。
    let mut h = connected_harness(&[]);
    h.module.ingest_increment(&inc(22, 999, false), &mut h.sink);
    assert_eq!(
        h.module.cursor_for(channel_id(22)),
        Some(0),
        "冷启动真新 channel cursor 种子须为 0"
    );

    h.sink.clear();
    h.module.ingest_increment_end(None, &mut h.sink);

    // 主 resync 因 22 在 skip 集不发；B2 heal 判 cursor(0)<target(999)=stuck → 补发一次。
    assert_eq!(
        sync_http_count(&h.sink),
        1,
        "冷启动 skip channel 须经 B2 heal 补发 sync（cursor 0 < 水位 999 = stuck），got: {:?}",
        http_reqs(&h.sink)
    );
    assert_eq!(
        first_sync_from_seq(&h.sink),
        0,
        "B2 heal 补发的 sync from_seq 必须 == 0（拉全量离线事件）"
    );
}

// ─── ⑤ need_sync=true 落后 channel 不被 heal 重复发（inflight 守卫去重）────────────

#[test]
fn heal_does_not_double_send_need_sync_true_channel() {
    // channel=13 need_sync=true cursor=20 < 水位 999：主 resync 已发 1 次。
    let mut h = connected_harness(&[]);
    h.module.register_channel(channel_id(13), 20);
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
