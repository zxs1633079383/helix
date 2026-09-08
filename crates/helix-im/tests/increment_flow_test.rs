//! B-rest 分片B 验收：increment 流程（push 模型）回放测试
//!
//! 回归锚: B2 —— increment HTTP 触发 + increment_channel 落地 + global-end→sync。
//!
//! ## 验收场景（设计文档 §3 理想扭转流程）
//!
//! 1. hello 握手 → 触发 increment HTTP（POST /channels/load/increment，**非** /sync/notify resync）。
//! 2. 喂 increment_channel → 注册 channel + cursor 种子并缓冲持久化/投影。
//! 3. increment_channel_end(None=global) → 启动独立 sync，并封出同批 Persist；ack 后发布最终投影。

mod common;
use common::*;

use helix_core::effect::TransportId;
use helix_core::{Effect, Tick};
use helix_im::state::Seq;
use helix_im::sync_session::IncrementChannel;

// ─── 辅助：从 sink 取真 Http req（断言 method/url）────────────────────────────

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

fn inc(channel_num: u64, last_event_seq: u64, need_sync: bool, raw: &str) -> IncrementChannel {
    IncrementChannel {
        channel_id: channel_id(channel_num),
        last_event_seq: Seq(last_event_seq),
        need_sync,
        raw: bytes::Bytes::from(raw.to_string()),
    }
}

// ─── ① hello → increment HTTP（POST /channels/load/increment，非 resync）────────

#[test]
fn hello_triggers_increment_http_not_resync() {
    // 预注册 channel=1（已知游标 0）；启动投影尚未完成时，hello 仍按既有闸门发送空 cursor 快照。
    let mut h = Harness::new(&[1]);
    h.start(0);
    h.step(Tick::Connected(TransportId::from_raw(0)), 100);

    let hello_effects = h.step(hello_inbound("CONN_INC"), 200);

    assert!(
        http_reqs(&h.sink).is_empty(),
        "hello must wait for the local message watermark scan before HTTP"
    );
    let timestamp_scan_corr =
        message_timestamp_scan_corr(&h.sink).expect("hello should emit a message watermark scan");
    h.step(
        port_reply_with_body(
            timestamp_scan_corr,
            bytes::Bytes::from_static(br#"[{"update_at":1726000123}]"#),
        ),
        210,
    );

    // message scan 回报后应恰好一个 Http，是 increment 触发（POST .../channels/load/increment），不是 sync/notify。
    let reqs = http_reqs(&h.sink);
    assert_eq!(
        reqs.len(),
        1,
        "hello 后应只发 increment HTTP（不再逐 channel resync），got: {:?}",
        reqs
    );
    let (method, url) = &reqs[0];
    assert_eq!(method, "POST", "increment 触发须 POST");
    assert!(
        url.ends_with("/channels/load/increment"),
        "hello 后须触发 increment HTTP 而非 sync/notify resync，got url: {}",
        url
    );
    assert!(
        !url.contains("sync/notify"),
        "hello 后不得直接 resync（sync/notify），got url: {}",
        url
    );
    let body = h.last_http_body().expect("increment HTTP body");
    assert_eq!(body["timestamp"], 1_726_000_123);
    assert_eq!(body["cursors"].as_array().map(Vec::len), Some(0));

    // hello 只发布连接事实；channels:loaded 必须等 global end 的本地事务成功。
    let emit_count = hello_effects
        .iter()
        .filter(|e| **e == EffectKind::Emit)
        .count();
    assert_eq!(
        emit_count, 1,
        "hello 只能 emit established，got {emit_count} emits"
    );
    assert!(!h.sink.as_slice().iter().any(|effect| {
        matches!(effect, Effect::Emit { event }
            if String::from_utf8_lossy(event.0.as_ref()).contains("im:channels:loaded"))
    }));

    // increment HTTP 须带 connectionId 身份头。
    let req = h
        .sink
        .as_slice()
        .iter()
        .find_map(|e| match e {
            Effect::Http { req, .. } => Some(req),
            _ => None,
        })
        .expect("应有 increment Http");
    assert!(
        req.headers
            .iter()
            .any(|(k, v)| k == "connectionId" && v == "CONN_INC"),
        "increment HTTP 须带 connectionId 身份头，got: {:?}",
        req.headers
    );
}

/// 同一物理连接的重复 hello 不得清空已收到的增量或创建第二个恢复请求。
#[test]
fn duplicate_hello_preserves_increment_and_original_scan() {
    let mut h = Harness::new(&[]);
    h.start(0);
    h.step(Tick::Connected(TransportId::from_raw(0)), 100);
    h.step(hello_inbound("CONN_DEDUP"), 200);
    let original = message_timestamp_scan_corr(&h.sink).expect("initial watermark scan");
    h.module
        .ingest_increment(&inc(77, 42, false, "{}"), &mut h.sink);
    h.step(hello_inbound("CONN_DEDUP"), 201);
    assert!(
        h.sink.as_slice().is_empty(),
        "duplicate hello emitted effects"
    );
    assert!(
        h.module.increment_fetched_contains(channel_id(77)),
        "duplicate hello reset increment batch"
    );
    h.step(
        port_reply_with_body(original, bytes::Bytes::from_static(b"[]")),
        202,
    );
    assert_eq!(
        http_reqs(&h.sink).len(),
        1,
        "original recovery must remain live"
    );
    h.step(hello_inbound("CONN_DEDUP"), 203);
    assert!(
        h.sink.as_slice().is_empty(),
        "duplicate hello restarted submitted recovery"
    );
    h.step(hello_inbound("CONN_NEW"), 204);
    assert!(
        message_timestamp_scan_corr(&h.sink).is_some(),
        "new connection must recover"
    );
    assert!(
        !h.module.increment_fetched_contains(channel_id(77)),
        "new connection must reset old batch"
    );
}

/// 断线后即使服务端复用连接 ID，也必须启动新恢复并丢弃旧连接的迟到回包。
#[test]
fn reconnect_hello_recovers_and_old_connection_scan_is_ignored() {
    let mut h = Harness::new(&[]);
    h.start(0);
    h.step(Tick::Connected(TransportId::from_raw(0)), 100);
    h.step(hello_inbound("CONN_OLD"), 101);
    let old = message_timestamp_scan_corr(&h.sink).expect("old scan");
    h.step(Tick::Disconnected(TransportId::from_raw(0)), 102);
    h.step(Tick::Connected(TransportId::from_raw(0)), 103);
    h.step(hello_inbound("CONN_NEW"), 104);
    let current = message_timestamp_scan_corr(&h.sink).expect("new scan");
    h.step(
        port_reply_with_body(old, bytes::Bytes::from_static(b"[]")),
        105,
    );
    assert!(h.sink.as_slice().is_empty());
    h.step(
        port_reply_with_body(current, bytes::Bytes::from_static(b"[]")),
        106,
    );
    assert_eq!(http_reqs(&h.sink).len(), 1);
    h.step(Tick::Disconnected(TransportId::from_raw(0)), 107);
    h.step(Tick::Connected(TransportId::from_raw(0)), 108);
    h.step(hello_inbound("CONN_NEW"), 109);
    assert!(message_timestamp_scan_corr(&h.sink).is_some());
}

// ─── ② increment_channel → 注册 channel + cursor 种子 + emit im:channel:increment ─

#[test]
fn increment_channel_registers_and_seeds_cursor() {
    let mut h = Harness::new(&[]);
    h.start(0);
    h.step(Tick::Connected(TransportId::from_raw(0)), 100);
    h.step(hello_inbound("CONN_INC"), 200);

    // 喂一个全新 channel=77 的 increment_channel，LastEventSeq=42（服务端水位）。
    h.sink.clear();
    h.module.ingest_increment(
        &inc(77, 42, true, r#"{"mentionList":[],"urgentPostList":[]}"#),
        &mut h.sink,
    );

    // 修 todo#1：冷启动真新 channel 已注册，但内存 cursor 种子 = 0（本地未确认任何 event），
    // **不是** LastEventSeq(42)。误种水位 → from_seq=cursor=max → Go no_change → 离线 backfill 永不触发。
    let cursor = h.module.cursor_for(channel_id(77));
    assert_eq!(
        cursor,
        Some(0),
        "冷启动真新 channel cursor 种子须为 0（本地确认水位），不得用 LastEventSeq（服务端水位）"
    );

    // 记入 increment_fetched。
    assert!(
        h.module.increment_fetched_contains(channel_id(77)),
        "increment 须记入 increment_fetched 集"
    );

    // G-S1：逐帧不直接 emit；global end 将 channel/member/cursor 合批，ack 后再透传 raw。
    let saw_increment_emit = h.sink.as_slice().iter().any(|e| {
        matches!(e, Effect::Emit { event }
        if String::from_utf8_lossy(event.0.as_ref()).contains("im:channel:increment"))
    });
    assert!(
        !saw_increment_emit,
        "increment 持久化 ack 前不得 emit im:channel:increment"
    );
}

/// 幂等：已存在的 channel（cursor 已推进）不被 increment 种子回退。
#[test]
fn increment_channel_does_not_rewind_existing_cursor() {
    let mut h = Harness::new(&[]);
    h.module.register_channel(channel_id(5), 100);
    h.start(0);
    h.step(Tick::Connected(TransportId::from_raw(0)), 100);
    h.step(hello_inbound("CONN_INC"), 200);

    // increment 带更低的 LastEventSeq=20，不应回退已有 cursor=100。
    h.sink.clear();
    h.module
        .ingest_increment(&inc(5, 20, true, "{}"), &mut h.sink);
    assert_eq!(
        h.module.cursor_for(channel_id(5)),
        Some(100),
        "increment 不得回退已有 channel cursor"
    );
}

// ─── ③ increment_channel_end(None global) → 对增量群产 sync Http ────────────────

#[test]
fn global_increment_end_syncs_fetched_minus_skip() {
    let mut h = Harness::new(&[]);
    h.start(0);
    h.step(Tick::Connected(TransportId::from_raw(0)), 100);
    h.step(hello_inbound("CONN_INC"), 200);

    // 三个增量群：10(need_sync=true) / 11(need_sync=true) / 12(need_sync=false→skip 主 resync)。
    // 修 todo#1 后：三者皆冷启动真新 channel → cursor 种子=0，increment_target=各自 lastEventSeq。
    h.sink.clear();
    h.module
        .ingest_increment(&inc(10, 5, true, "{}"), &mut h.sink);
    h.module
        .ingest_increment(&inc(11, 8, true, "{}"), &mut h.sink);
    h.module
        .ingest_increment(&inc(12, 3, false, "{}"), &mut h.sink);

    // global end（None）→ 主 resync 对 (fetched − skip) = {10, 11} 各发一次；
    // B2 heal 再对 12 补发（cursor 0 < target 3 = 真有离线事件被 need_sync=false 漏掉）→ 共 3 次。
    // 修前 12 的 cursor 误种子=3 → heal 的 cursor<target 假 → 12 离线事件被静默吞掉（backfill 缺口）。
    h.sink.clear();
    h.module.ingest_increment_end(None, &mut h.sink);

    let reqs = http_reqs(&h.sink);
    assert_eq!(
        reqs.len(),
        3,
        "global-end：主 resync {{10,11}} 各一次 + B2 heal 对 need_sync=false 但落后水位的 12 补一次 = 3，got: {:?}",
        reqs
    );
    for (method, url) in &reqs {
        assert_eq!(method, "POST", "sync/notify 须 POST（设计文档 §1.4）");
        assert!(
            url.ends_with("/channel/sync/notify"),
            "global-end 须发 sync/notify，got url: {}",
            url
        );
    }
}

/// per-channel end（Some）→ emit im:channel:update，不发 sync。
#[test]
fn subtopic_increment_end_emits_update_no_sync() {
    let mut h = Harness::new(&[]);
    h.start(0);
    h.step(Tick::Connected(TransportId::from_raw(0)), 100);
    h.step(hello_inbound("CONN_INC"), 200);

    h.sink.clear();
    h.module
        .ingest_increment_end(Some(channel_id(99)), &mut h.sink);

    // 无 Http（不 sync）。
    assert!(
        http_reqs(&h.sink).is_empty(),
        "子 topic end 不应发 sync Http"
    );
    // emit im:channel:update。
    let saw_update = h.sink.as_slice().iter().any(|e| {
        matches!(e, Effect::Emit { event }
        if String::from_utf8_lossy(event.0.as_ref()).contains("im:channel:update"))
    });
    assert!(saw_update, "子 topic end 须 emit im:channel:update");
}

/// 会话边界 reset：第二次 hello 起点清空上一批 increment_fetched（§7 跨会话残留）。
#[test]
fn second_hello_resets_increment_batch() {
    let mut h = Harness::new(&[]);
    h.start(0);
    h.step(Tick::Connected(TransportId::from_raw(0)), 100);
    h.step(hello_inbound("CONN_A"), 200);
    h.module
        .ingest_increment(&inc(33, 1, true, "{}"), &mut h.sink);
    assert!(h.module.increment_fetched_contains(channel_id(33)));

    // 第二次 hello（重连）→ 应清空上一会话残留集。
    h.step(hello_inbound("CONN_B"), 300);
    assert!(
        !h.module.increment_fetched_contains(channel_id(33)),
        "重连 hello 须 reset increment 批次集（防 sync 风暴）"
    );
}
