//! E4 修复验收测试：scan_corr PortReply 路由
//!
//! ## 验收场景
//!
//! on_start 发出 Persist{Scan(channel_event_cursor)}，PortReply 回来时：
//! 1. 解析 rows（channel_id TEXT + last_event_seq INTEGER）→ register_channel
//! 2. 再扫描 channel 投影，过滤 delete_at/is_remove 后才发补偿 Http
//! 3. 解析失败降级：log warn，不崩，空 channel 集启动仍可用
//!
//! ## schema 契约
//!
//! cursor 扫描目标表：`channel_event_cursor`（列：channel_id TEXT PRIMARY KEY,
//! last_event_seq INTEGER）
//! 写路径：advance_cursor → MonotonicUpsert{channel_id=channel_id.to_string(), value=seq}
//! 读路径：engine_loop rows_to_reply_bytes → JSON
//! `[{{"channel_id":"<channel-id>","last_event_seq":N}},...]`

mod common;
use common::*;

use helix_core::effect::{Correlation, TransportId};
use helix_core::{Effect, Tick};

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

/// 取当前 step 中新增的 Persist correlation，区分 cursor 与 channel 投影扫描。
fn persist_corrs(sink: &helix_core::EffectSink) -> Vec<u64> {
    sink.as_slice()
        .iter()
        .filter_map(|effect| match effect {
            Effect::Persist { corr, .. } => Some(corr.raw()),
            _ => None,
        })
        .collect()
}

// ─── 核心验收：scan 3 行 → 3 个 channel 注册 + Connected 后 3 个 sync Http ──────

#[test]
fn scan_reply_registers_channels_and_syncs_on_connected() {
    let mut h = Harness::new(&[]);
    let start_effects = h.start(0);

    // 找到 on_start 发出的 Scan Persist corr
    let scan_persist_corr = start_effects
        .iter()
        .find_map(|e| {
            if let EffectKind::Persist(c) = e {
                Some(*c)
            } else {
                None
            }
        })
        .expect("on_start should emit Persist(Scan)");

    // driver 回报 scan 结果：3 行（channel 101, 102, 103）
    let scan_body = scan_reply(&[(101, 50), (102, 100), (103, 0)]);
    let scan_port_reply = port_reply_with_body(scan_persist_corr, scan_body);

    // 在连接之前先回报 scan（尚未 Connected）
    let scan_effects = h.step(scan_port_reply, 100);
    // cursor scan reply 时尚未连接，不应产生 Http sync；随后先完成 channel 投影过滤。
    assert!(
        !scan_effects
            .iter()
            .any(|e| matches!(e, EffectKind::Http(_))),
        "scan reply before Connected should NOT emit Http sync, got: {:?}",
        scan_effects
    );
    let projection_scan_corr = persist_corrs(&h.sink)
        .into_iter()
        .next()
        .expect("cursor scan should schedule channel projection scan");
    let projection_effects = h.step(
        port_reply_with_body(
            projection_scan_corr,
            channel_projection_scan_reply(&[(101, false, 50), (102, false, 100), (103, false, 0)]),
        ),
        150,
    );
    assert!(
        !projection_effects
            .iter()
            .any(|e| matches!(e, EffectKind::Http(_))),
        "projection scan before Connected should NOT emit Http sync, got: {:?}",
        projection_effects
    );

    // A4：Connected 只置 Connecting，不立即 resync（握手前 resync 会被 Go 401）
    let connected_effects = h.step(Tick::Connected(TransportId::from_raw(0)), 200);
    assert_eq!(
        connected_effects
            .iter()
            .filter(|e| matches!(e, EffectKind::Http(_)))
            .count(),
        0,
        "A4: Connected 不再立即 resync（等 hello），got: {:?}",
        connected_effects
    );

    // A4+B-rest：收到 hello → 握手完成 → 触发 1 个 increment HTTP（带 3 个已加载 channel 游标），
    // 不再逐 channel resync。scan 载入 channel 的正确性由 scan_reply_loads_correct_cursors 覆盖。
    h.step(hello_inbound("CONN_1"), 300);
    let reqs = http_reqs(&h.sink);
    assert_eq!(
        reqs.len(),
        1,
        "hello 后应发 1 个 increment HTTP（带 3 channel 游标），got: {:?}",
        reqs
    );
    assert_eq!(reqs[0].0, "POST");
    assert!(
        reqs[0].1.ends_with("/channels/load/increment"),
        "hello 须触发 increment HTTP 而非逐 channel sync，got: {}",
        reqs[0].1
    );
}

#[test]
fn scan_reply_after_connected_immediately_triggers_sync() {
    let mut h = Harness::new(&[]);
    let start_effects = h.start(0);
    let scan_persist_corr = start_effects
        .iter()
        .find_map(|e| {
            if let EffectKind::Persist(c) = e {
                Some(*c)
            } else {
                None
            }
        })
        .expect("on_start should emit Persist(Scan)");

    // A4：先 Connected（→Connecting）再 hello（→Connected 握手完成）。
    // B-rest：即使 channels 空，hello 也触发 1 个 increment HTTP（空 cursors）——旧「空→0 Http」前提不再成立。
    h.step(Tick::Connected(TransportId::from_raw(0)), 100);
    h.step(hello_inbound("CONN_1"), 150);
    let hello_reqs = http_reqs(&h.sink);
    assert_eq!(
        hello_reqs.len(),
        1,
        "B-rest：hello 即使 channels 空也发 1 个 increment HTTP，got: {:?}",
        hello_reqs
    );
    assert!(
        hello_reqs[0].1.ends_with("/channels/load/increment"),
        "hello 须触发 increment HTTP，got: {}",
        hello_reqs[0].1
    );

    // 然后 cursor scan 回报（已握手完成/Connected）；删除过滤完成后再补发 bootstrap/sync。
    let scan_body = scan_reply(&[(1, 10), (2, 20), (3, 30)]);
    let scan_port_reply = port_reply_with_body(scan_persist_corr, scan_body);
    h.step(scan_port_reply, 200);
    let projection_scan_corr = persist_corrs(&h.sink)
        .into_iter()
        .next()
        .expect("cursor scan should schedule channel projection scan");
    let projection_effects = h.step(
        port_reply_with_body(
            projection_scan_corr,
            channel_projection_scan_reply(&[(1, false, 10), (2, false, 20), (3, false, 30)]),
        ),
        250,
    );
    let reqs = http_reqs(&h.sink);
    let increment_count = reqs
        .iter()
        .filter(|(_, url)| url.ends_with("/channels/load/increment"))
        .count();
    let sync_count = reqs
        .iter()
        .filter(|(_, url)| url.ends_with("/channel/sync/notify"))
        .count();
    assert_eq!(
        (increment_count, sync_count),
        (1, 3),
        "channel projection scan should retry increment once and emit 3 sync Http, got reqs={reqs:?}, effects={projection_effects:?}"
    );
}

#[test]
fn scan_reply_after_empty_hello_retries_increment_with_loaded_cursors() {
    let mut h = Harness::new(&[]);
    let start_effects = h.start(0);
    let scan_persist_corr = start_effects
        .iter()
        .find_map(|e| {
            if let EffectKind::Persist(c) = e {
                Some(*c)
            } else {
                None
            }
        })
        .expect("on_start should emit Persist(Scan)");

    h.step(Tick::Connected(TransportId::from_raw(0)), 100);
    h.step(hello_inbound("CONN_1"), 150);
    let hello_body = h
        .last_http_body()
        .expect("hello should emit increment HTTP");
    assert_eq!(
        hello_body["cursors"].as_array().map(Vec::len),
        Some(0),
        "pre-scan hello reproduces the empty-cursor bootstrap race"
    );

    let scan_body = scan_reply(&[(1, 10), (2, 20), (3, 30)]);
    h.step(port_reply_with_body(scan_persist_corr, scan_body), 200);
    let projection_scan_corr = persist_corrs(&h.sink)
        .into_iter()
        .next()
        .expect("cursor scan should schedule channel projection scan");
    let reply_effects = h.step(
        port_reply_with_body(
            projection_scan_corr,
            channel_projection_scan_reply(&[(1, false, 10), (2, false, 20), (3, false, 30)]),
        ),
        250,
    );
    let reqs: Vec<_> = h
        .sink
        .as_slice()
        .iter()
        .filter_map(|e| match e {
            Effect::Http { req, .. } => Some(req),
            _ => None,
        })
        .collect();
    let retry = reqs
        .iter()
        .find(|req| req.url.ends_with("/channels/load/increment"))
        .expect("scan reply should retry bootstrap increment with loaded cursors");
    let retry_body: serde_json::Value =
        serde_json::from_slice(retry.body.as_ref().unwrap()).unwrap();
    let cursors = retry_body["cursors"]
        .as_array()
        .expect("retry cursors should be an array");
    assert_eq!(cursors.len(), 3, "retry must carry scanned channel cursors");
    assert_eq!(cursors[0]["channelId"], cid(1));
    assert_eq!(cursors[0]["fromSeq"], 10);
    assert!(
        reply_effects
            .iter()
            .any(|e| matches!(e, EffectKind::Http(_))),
        "scan reply should emit retry/sync Http effects"
    );
}

#[test]
fn channel_projection_scan_marks_deleted_channel_terminal() {
    let mut h = Harness::new(&[]);
    let start_effects = h.start(0);
    let cursor_scan_corr = start_effects
        .iter()
        .find_map(|effect| match effect {
            EffectKind::Persist(corr) => Some(*corr),
            _ => None,
        })
        .expect("on_start should emit cursor scan");

    h.step(
        port_reply_with_body(cursor_scan_corr, scan_reply(&[(7, 0)])),
        100,
    );
    let projection_scan_corr = persist_corrs(&h.sink)
        .into_iter()
        .next()
        .expect("cursor scan should schedule channel projection scan");
    let projection_effects = h.step(
        port_reply_with_body(
            projection_scan_corr,
            channel_projection_scan_reply(&[(7, true, 4)]),
        ),
        150,
    );

    assert_eq!(h.module.terminal_event_seq_for(channel_id(7)), Some(4));
    assert!(
        !http_reqs(&h.sink)
            .iter()
            .any(|(_, url)| url.ends_with("/channel/sync/notify")),
        "deleted channel must not emit startup sync, got effects={projection_effects:?}"
    );
}

#[test]
fn channel_projection_scan_error_keeps_startup_sync_fail_closed() {
    let mut h = Harness::new(&[]);
    let start_effects = h.start(0);
    let cursor_scan_corr = start_effects
        .iter()
        .find_map(|effect| match effect {
            EffectKind::Persist(corr) => Some(*corr),
            _ => None,
        })
        .expect("on_start should emit cursor scan");

    h.step(
        port_reply_with_body(cursor_scan_corr, scan_reply(&[(8, 12)])),
        100,
    );
    let projection_scan_corr = persist_corrs(&h.sink)
        .into_iter()
        .next()
        .expect("cursor scan should schedule channel projection scan");
    let effects = h.step(port_reply_err(projection_scan_corr), 150);

    assert!(
        !effects
            .iter()
            .any(|effect| matches!(effect, EffectKind::Http(_))),
        "projection scan failure must not emit compensating HTTP, got: {effects:?}"
    );
    h.step(Tick::Connected(TransportId::from_raw(0)), 200);
    h.step(hello_inbound("CONN_SCAN_ERROR"), 250);
    let body = h
        .last_http_body()
        .expect("hello should emit increment HTTP");
    assert_eq!(
        body["cursors"].as_array().map(Vec::len),
        Some(0),
        "failed projection scan must keep the active cursor snapshot gated"
    );
}

// ─── 降级：scan 回报失败，不崩，空 channel 集启动仍可用 ──────────────────────────

#[test]
fn scan_reply_error_degrades_gracefully() {
    let mut h = Harness::new(&[]);
    let start_effects = h.start(0);
    let scan_persist_corr = start_effects
        .iter()
        .find_map(|e| {
            if let EffectKind::Persist(c) = e {
                Some(*c)
            } else {
                None
            }
        })
        .expect("on_start should emit Persist(Scan)");

    // driver 回报 scan 失败
    let err_reply = Tick::PortReply {
        corr: Correlation::from_raw(scan_persist_corr),
        outcome: helix_core::tick::PortOutcome::Err(helix_core::tick::PortError::Http(500)),
    };
    // 不应 panic
    let effects = h.step(err_reply, 100);
    // scan 失败不产生 Http sync（空 channel 集）
    assert!(
        !effects.iter().any(|e| matches!(e, EffectKind::Http(_))),
        "scan error should not emit Http, got: {:?}",
        effects
    );

    // 后续 Connected 仍可工作（0 channel）
    let connected_effects = h.step(Tick::Connected(TransportId::from_raw(0)), 200);
    let http_count = connected_effects
        .iter()
        .filter(|e| matches!(e, EffectKind::Http(_)))
        .count();
    assert_eq!(
        http_count, 0,
        "after scan error, Connected with 0 channels should emit 0 Http"
    );
}

// ─── scan 结果 cursor 正确载入 ────────────────────────────────────────────────

#[test]
fn scan_reply_loads_correct_cursors() {
    let mut h = Harness::new(&[]);
    let start_effects = h.start(0);
    let scan_persist_corr = start_effects
        .iter()
        .find_map(|e| {
            if let EffectKind::Persist(c) = e {
                Some(*c)
            } else {
                None
            }
        })
        .expect("on_start should emit Persist(Scan)");

    // scan 回报 channel_id=42，cursor=99
    let scan_body = scan_reply(&[(42, 99)]);
    h.step(port_reply_with_body(scan_persist_corr, scan_body), 100);

    // 验证 channel 已注册且 cursor=99
    let cursor = h.module.cursor_for(channel_id(42));
    assert_eq!(
        cursor,
        Some(99),
        "scan should load cursor=99 for channel 42"
    );
}

// ─── 幂等：若 channel 已预注册，scan 不覆盖 ──────────────────────────────────────

#[test]
fn scan_reply_does_not_overwrite_existing_channel() {
    // 预注册 channel=1，cursor=200
    let mut h = Harness::new(&[]);
    h.module.register_channel(channel_id(1), 200);

    let start_effects = h.start(0);
    let scan_persist_corr = start_effects
        .iter()
        .find_map(|e| {
            if let EffectKind::Persist(c) = e {
                Some(*c)
            } else {
                None
            }
        })
        .expect("on_start should emit Persist(Scan)");

    // scan 回报 channel_id=1，cursor=50（更低）
    let scan_body = scan_reply(&[(1, 50)]);
    h.step(port_reply_with_body(scan_persist_corr, scan_body), 100);

    // cursor 应保持 200（entry().or_insert_with 幂等，不覆盖已有值）
    let cursor = h.module.cursor_for(channel_id(1));
    assert_eq!(
        cursor,
        Some(200),
        "scan should not overwrite pre-registered channel cursor"
    );
}
