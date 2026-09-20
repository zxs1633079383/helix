//! 生命周期测试（Phase B Commit 4）
//!
//! 验证：
//! - on_start：吐 Persist{Scan} + ScheduleTimer{ping}
//! - Tick::Connected：proactive resync（对所有注册 channel 各发 Http{sync/notify}）
//! - Tick::Disconnected：静默标记 offline（无 Effect）
//! - on_stop：取消所有 timer

mod common;
use common::*;

use helix_core::effect::TransportId;
use helix_core::{Effect, Tick};
use helix_im::state::Seq;
use helix_im::sync_session::IncrementChannel;

/// 收集本批所有 Http effect 的 (method, url)（断言 increment HTTP vs sync/notify）。
fn http_reqs(sink: &helix_core::EffectSink) -> Vec<(String, String)> {
    sink.as_slice()
        .iter()
        .filter_map(|e| match e {
            Effect::Http { req, .. } => Some((req.method.clone(), req.url.clone())),
            _ => None,
        })
        .collect()
}

/// 构造 IncrementChannel（数值 id → 26 字符）。
fn inc(channel_num: u64, last_event_seq: u64, need_sync: bool) -> IncrementChannel {
    IncrementChannel {
        channel_id: channel_id(channel_num),
        last_event_seq: Seq(last_event_seq),
        need_sync,
        unread_post_id: None,
        last_read_seq: None,
        projection_revision: None,
        raw: bytes::Bytes::from_static(b"{}"),
    }
}

/// 构造带删除投影标记的 IncrementChannel，覆盖 Go/SQLite 的字段命名形态。
fn deleted_inc(
    channel_num: u64,
    last_event_seq: u64,
    need_sync: bool,
    field: &str,
) -> IncrementChannel {
    let value = if field == "deleteAt" {
        serde_json::json!(1)
    } else {
        serde_json::json!(true)
    };
    let raw = serde_json::json!({field: value});
    IncrementChannel {
        channel_id: channel_id(channel_num),
        last_event_seq: Seq(last_event_seq),
        need_sync,
        unread_post_id: None,
        last_read_seq: None,
        projection_revision: None,
        raw: bytes::Bytes::from(serde_json::to_vec(&raw).unwrap()),
    }
}

/// 完成 on_start 的 cursor/channel 双扫描；hello 只有在这个 scope barrier 后才可读水位。
fn complete_startup_scope_scans(h: &mut Harness, cursors: &[(u64, u64)]) {
    let start_effects = h.start(0);
    let cursor_scan_corr = start_effects
        .iter()
        .find_map(|effect| match effect {
            EffectKind::Persist(corr) => Some(*corr),
            _ => None,
        })
        .expect("on_start should emit cursor scan");

    let cursor_effects = h.step(
        port_reply_with_body(cursor_scan_corr, scan_reply(cursors)),
        100,
    );
    let projection_scan_corr = cursor_effects
        .iter()
        .find_map(|effect| match effect {
            EffectKind::Persist(corr) => Some(*corr),
            _ => None,
        })
        .expect("cursor scan should schedule channel projection scan");
    let projection_rows: Vec<_> = cursors
        .iter()
        .map(|&(channel_id, cursor)| (channel_id, false, cursor))
        .collect();
    h.step(
        port_reply_with_body(
            projection_scan_corr,
            channel_projection_scan_reply(&projection_rows),
        ),
        150,
    );
}

// ─── on_start 行为 ────────────────────────────────────────────────────────────

#[test]
fn on_start_emits_scan_persist_and_ping_timer() {
    let mut h = Harness::new(&[]);
    let effects = h.start(0);

    // 应包含 Persist（Scan cursor）
    assert!(
        effects.iter().any(|e| matches!(e, EffectKind::Persist(_))),
        "on_start should emit Persist(Scan), got: {:?}",
        effects
    );
    // 应包含 ScheduleTimer（ping）
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, EffectKind::ScheduleTimer(_))),
        "on_start should emit ScheduleTimer(ping), got: {:?}",
        effects
    );
    // 不应产出 Http（on_start 不主动 sync，等 Connected 后才 proactive resync）
    assert!(
        !effects.iter().any(|e| matches!(e, EffectKind::Http(_))),
        "on_start should NOT emit Http, got: {:?}",
        effects
    );
}

#[test]
fn on_start_with_channels_still_no_http() {
    // 即使预注册了 channel，on_start 也不应发 Http（等 Connected 触发 proactive resync）
    let mut h = Harness::new(&[1, 2, 3]);
    let effects = h.start(0);
    assert!(
        !effects.iter().any(|e| matches!(e, EffectKind::Http(_))),
        "on_start with channels should NOT emit Http (proactive resync is in Connected), got: {:?}",
        effects
    );
}

// ─── Tick::Connected → proactive resync ──────────────────────────────────────

#[test]
fn connected_triggers_proactive_resync_for_each_channel() {
    let mut h = Harness::new(&[1, 2, 3]);
    complete_startup_scope_scans(&mut h, &[(1, 0), (2, 0), (3, 0)]);

    // A4：Connected 只置 Connecting；B-rest：hello 后触发 1 个 increment HTTP（非逐 channel resync）
    h.step(Tick::Connected(TransportId::from_raw(0)), 500);
    h.step(hello_inbound("CONN_1"), 600);
    let timestamp_scan_corr =
        message_timestamp_scan_corr(&h.sink).expect("hello should emit a message watermark scan");
    h.step(
        port_reply_with_body(timestamp_scan_corr, bytes::Bytes::from_static(br#"[]"#)),
        610,
    );
    let reqs = http_reqs(&h.sink);
    assert_eq!(
        reqs.len(),
        1,
        "B-rest：hello 后应只发 1 个 increment HTTP（不再逐 channel resync），got: {:?}",
        reqs
    );
    assert_eq!(reqs[0].0, "POST", "increment 触发须 POST");
    assert!(
        reqs[0].1.ends_with("/channels/load/increment"),
        "hello 须触发 increment HTTP 而非 sync/notify，got: {}",
        reqs[0].1
    );

    // 真 sync 在 increment_channel ×3 + increment_end(None=global) 后，对 3 个增量群各发一次 sync POST。
    h.sink.clear();
    for ch in [1u64, 2, 3] {
        h.module.ingest_increment(&inc(ch, 0, true), &mut h.sink);
    }
    h.sink.clear();
    h.module.ingest_increment_end(None, &mut h.sink);
    let syncs = http_reqs(&h.sink);
    assert_eq!(
        syncs.len(),
        3,
        "global-end 须对 3 个增量群各发一次 sync POST，got: {:?}",
        syncs
    );
    for (method, url) in &syncs {
        assert_eq!(method, "POST", "sync/notify 须 POST（设计文档 §1.4）");
        assert!(
            url.ends_with("/channel/sync/notify"),
            "global-end 须发 sync/notify，got: {}",
            url
        );
    }
}

/// 从 sink 抽取所有 sync/notify Http Effect 的 channelId（按 Effect 产出顺序）。
fn sync_notify_channel_ids(sink: &helix_core::EffectSink) -> Vec<String> {
    sink.as_slice()
        .iter()
        .filter_map(|e| match e {
            Effect::Http { req, .. } if req.url.ends_with("/channel/sync/notify") => {
                let body = req.body.as_ref()?;
                let v: serde_json::Value = serde_json::from_slice(body).ok()?;
                v["cursors"][0]["channelId"].as_str().map(str::to_string)
            }
            _ => None,
        })
        .collect()
}

/// NS-1 回归：global-end resync 必须按 `increment_channel` 的确定性到达顺序产出。
///
/// 回归锚: NS-1（北极星符合性 review wf_69cf287f，GOAL §1 确定性可回放）。
/// 防 `increment_fetched`(HashSet) 迭代序逐进程随机，同时保留服务端的新鲜度排序，
/// 让最近活跃频道优先占用 sync 窗口。
#[test]
fn increment_end_resync_preserves_deterministic_delivery_order() {
    let mut h = Harness::new(&[3, 1, 2]); // 非排序注册顺序
    h.start(0);
    h.step(Tick::Connected(TransportId::from_raw(0)), 500);
    h.step(hello_inbound("CONN_NS1"), 600);

    h.sink.clear();
    for ch in [3u64, 1, 2] {
        h.module.ingest_increment(&inc(ch, 0, true), &mut h.sink);
    }
    h.sink.clear();
    h.module.ingest_increment_end(None, &mut h.sink);

    let ids = sync_notify_channel_ids(&h.sink);
    assert_eq!(
        ids,
        vec![cid(3), cid(1), cid(2)],
        "NS-1: resync sync/notify 必须保留 increment 到达顺序，got {:?}",
        ids
    );
}

#[test]
fn deleted_increment_channel_is_terminal_before_global_end() {
    let mut h = Harness::new(&[]);
    let deleted = deleted_inc(41, 7, true, "deleteAt");
    h.module.ingest_increment(&deleted, &mut h.sink);
    assert_eq!(
        h.module.terminal_event_seq_for(channel_id(41)),
        Some(7),
        "deleteAt increment must close the in-memory channel before batch end"
    );

    h.sink.clear();
    h.module.ingest_increment_end(None, &mut h.sink);
    assert_eq!(
        http_reqs(&h.sink)
            .iter()
            .filter(|(_, url)| url.ends_with("/channel/sync/notify"))
            .count(),
        0,
        "deleted increment must not enter global-end resync"
    );

    let removed = deleted_inc(42, 9, true, "isRemove");
    h.module.ingest_increment(&removed, &mut h.sink);
    assert_eq!(
        h.module.terminal_event_seq_for(channel_id(42)),
        Some(9),
        "isRemove increment must close the in-memory channel"
    );
}

#[test]
fn terminal_channel_is_omitted_from_hello_increment_cursors() {
    let mut h = Harness::new(&[]);
    h.module
        .ingest_increment(&deleted_inc(43, 5, true, "deleteAt"), &mut h.sink);
    complete_startup_scope_scans(&mut h, &[]);
    h.step(Tick::Connected(TransportId::from_raw(0)), 500);
    h.step(hello_inbound("CONN_TERMINAL"), 600);
    let timestamp_scan_corr =
        message_timestamp_scan_corr(&h.sink).expect("hello should emit a message watermark scan");
    h.step(
        port_reply_with_body(timestamp_scan_corr, bytes::Bytes::from_static(br#"[]"#)),
        610,
    );

    let body = h
        .last_http_body()
        .expect("hello should emit increment HTTP");
    assert_eq!(
        body["cursors"].as_array().map(Vec::len),
        Some(0),
        "terminal channel must not be sent back as an active increment cursor"
    );
}

#[test]
fn connected_with_no_channels_emits_no_http() {
    let mut h = Harness::new(&[]);
    h.start(0);
    let connected_effects = h.step(Tick::Connected(TransportId::from_raw(0)), 500);
    let http_count = connected_effects
        .iter()
        .filter(|e| matches!(e, EffectKind::Http(_)))
        .count();
    assert_eq!(
        http_count, 0,
        "Connected with no channels should emit no Http, got: {:?}",
        connected_effects
    );
}

#[test]
fn connected_each_channel_gets_different_corr() {
    let mut h = Harness::new(&[10, 20, 30]);
    complete_startup_scope_scans(&mut h, &[(10, 0), (20, 0), (30, 0)]);
    h.step(Tick::Connected(TransportId::from_raw(0)), 500);

    // B-rest：hello 只发 1 个 increment HTTP（corr 唯一性移到 global-end sync 路径验证）
    h.step(hello_inbound("CONN_1"), 600);
    let timestamp_scan_corr =
        message_timestamp_scan_corr(&h.sink).expect("hello should emit a message watermark scan");
    let effects = h.step(
        port_reply_with_body(timestamp_scan_corr, bytes::Bytes::from_static(br#"[]"#)),
        610,
    );
    let hello_http = effects
        .iter()
        .filter(|e| matches!(e, EffectKind::Http(_)))
        .count();
    assert_eq!(
        hello_http, 1,
        "hello 只发 1 个 increment HTTP, got: {:?}",
        effects
    );

    // global-end sync：3 个增量群 → 3 个 sync Http，各 corr 互不相同。
    h.sink.clear();
    for ch in [10u64, 20, 30] {
        h.module.ingest_increment(&inc(ch, 0, true), &mut h.sink);
    }
    h.sink.clear();
    h.module.ingest_increment_end(None, &mut h.sink);
    let sync_corrs: Vec<u64> = h
        .sink
        .as_slice()
        .iter()
        .filter_map(|e| match e {
            Effect::Http { corr, .. } => Some(corr.raw()),
            _ => None,
        })
        .collect();
    assert_eq!(
        sync_corrs.len(),
        3,
        "global-end 应对 3 群各发一个 sync Http"
    );
    let unique: std::collections::HashSet<u64> = sync_corrs.iter().copied().collect();
    assert_eq!(unique.len(), 3, "each sync Http should have a unique corr");
}

// ─── Tick::Disconnected → 静默 + 状态标记 ────────────────────────────────────

#[test]
fn disconnected_produces_no_effects() {
    let mut h = Harness::new(&[1]);
    h.start(0);
    // 先连接
    h.step(Tick::Connected(TransportId::from_raw(0)), 500);
    // 再断线
    let disconnect_effects = h.step(Tick::Disconnected(TransportId::from_raw(0)), 1000);
    assert!(
        disconnect_effects.is_empty(),
        "Disconnected should produce no effects, got: {:?}",
        disconnect_effects
    );
}

#[test]
fn disconnected_then_reconnect_triggers_new_resync() {
    // 断线后重连，Connected 再次触发 proactive resync
    let mut h = Harness::new(&[1, 2]);
    complete_startup_scope_scans(&mut h, &[(1, 0), (2, 0)]);
    h.step(Tick::Connected(TransportId::from_raw(0)), 500);
    h.step(hello_inbound("CONN_1"), 600); // 首次握手
    let first_scan_corr = message_timestamp_scan_corr(&h.sink)
        .expect("first hello should emit a message watermark scan");
    h.step(
        port_reply_with_body(first_scan_corr, bytes::Bytes::from_static(br#"[]"#)),
        610,
    );
    h.step(Tick::Disconnected(TransportId::from_raw(0)), 1000);

    // 第二次连接 + 重新握手（hello 带新 connectionId）→ B-rest：1 个 increment HTTP
    h.step(Tick::Connected(TransportId::from_raw(0)), 2000);
    h.step(hello_inbound("CONN_2"), 2100);
    let second_scan_corr = message_timestamp_scan_corr(&h.sink)
        .expect("reconnect hello should emit a message watermark scan");
    h.step(
        port_reply_with_body(second_scan_corr, bytes::Bytes::from_static(br#"[]"#)),
        2110,
    );
    let reqs = http_reqs(&h.sink);
    assert_eq!(
        reqs.len(),
        1,
        "重连 hello 后应发 1 个 increment HTTP（重连 sync 走 increment_end），got: {:?}",
        reqs
    );
    assert!(
        reqs[0].1.ends_with("/channels/load/increment"),
        "重连须触发 increment HTTP，got: {}",
        reqs[0].1
    );

    // 重连 sync 走 increment_end(None)：喂 2 群 + global-end → 2 个 sync POST。
    h.sink.clear();
    for ch in [1u64, 2] {
        h.module.ingest_increment(&inc(ch, 0, true), &mut h.sink);
    }
    h.sink.clear();
    h.module.ingest_increment_end(None, &mut h.sink);
    let syncs = http_reqs(&h.sink);
    assert_eq!(
        syncs.len(),
        2,
        "重连 increment_end 后应对 2 群发 2 个 sync POST，got: {:?}",
        syncs
    );
}

// ─── ping timer 跨连接状态测试 ────────────────────────────────────────────────

#[test]
fn ping_timer_fires_only_when_connected() {
    let mut h = Harness::new(&[]);
    let start_effects = h.start(0);
    let ping_timer = start_effects
        .iter()
        .find_map(|e| {
            if let EffectKind::ScheduleTimer(id) = e {
                Some(*id)
            } else {
                None
            }
        })
        .expect("should have ping timer from on_start");

    // Disconnected 状态下 ping timer 触发 → 不应吐 Send 帧（只 re-arm timer）
    let disconnect_effects = h.step(Tick::Disconnected(TransportId::from_raw(0)), 500);
    assert!(disconnect_effects.is_empty());

    let timer_effects = h.step(timer_tick(ping_timer), 8001);
    // 应 re-arm（ScheduleTimer）
    assert!(
        timer_effects
            .iter()
            .any(|e| matches!(e, EffectKind::ScheduleTimer(_))),
        "ping timer should re-arm in disconnected state"
    );
    // 不应发 Send（未连接）
    assert!(
        !timer_effects.iter().any(|e| matches!(e, EffectKind::Send)),
        "ping timer should NOT emit Send when disconnected"
    );
}

#[test]
fn ping_timer_fires_send_when_connected() {
    let mut h = Harness::new(&[]);
    let start_effects = h.start(0);
    let ping_timer = start_effects
        .iter()
        .find_map(|e| {
            if let EffectKind::ScheduleTimer(id) = e {
                Some(*id)
            } else {
                None
            }
        })
        .expect("should have ping timer from on_start");

    // 连接建立 + hello 握手（A4：conn=Connected 才发 ping Send）
    h.step(Tick::Connected(TransportId::from_raw(0)), 500);
    h.step(hello_inbound("CONN_1"), 600);

    let timer_effects = h.step(timer_tick(ping_timer), 8001);
    // 应有 Send（ping 帧）
    assert!(
        timer_effects.iter().any(|e| matches!(e, EffectKind::Send)),
        "ping timer should emit Send when connected"
    );
    // 应 re-arm
    assert!(
        timer_effects
            .iter()
            .any(|e| matches!(e, EffectKind::ScheduleTimer(_))),
        "ping timer should re-arm"
    );
}
