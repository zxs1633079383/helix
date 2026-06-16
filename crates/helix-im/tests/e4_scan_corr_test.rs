//! E4 修复验收测试：scan_corr PortReply 路由
//!
//! ## 验收场景
//!
//! on_start 发出 Persist{Scan(_helix_monotonic)}，PortReply 回来时：
//! 1. 解析 rows（scope_key TEXT→u64 channel_id, value INTEGER→cursor）→ register_channel
//! 2. 若已 Connected，批量发 proactive sync Http（与 Connected 路径复用）
//! 3. 解析失败降级：log warn，不崩，空 channel 集启动仍可用
//!
//! ## schema 契约
//!
//! scan 目标表：`_helix_monotonic`（列：scope_key TEXT PRIMARY KEY, value INTEGER）
//! 写路径：advance_cursor → MonotonicUpsert{scope_key=channel_id.to_string(), value=seq}
//! 读路径：engine_loop rows_to_reply_bytes → JSON `[{"scope_key":"<u64>","value":N},...]`

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
    // scan reply 时尚未连接，不应产生 Http sync
    assert!(
        !scan_effects
            .iter()
            .any(|e| matches!(e, EffectKind::Http(_))),
        "scan reply before Connected should NOT emit Http sync, got: {:?}",
        scan_effects
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

    // 然后 scan 回报（已握手完成/Connected）→ E4 路径仍立即对 3 个 scan 载入 channel 发 sync Http
    let scan_body = scan_reply(&[(1, 10), (2, 20), (3, 30)]);
    let scan_port_reply = port_reply_with_body(scan_persist_corr, scan_body);
    let reply_effects = h.step(scan_port_reply, 200);
    let http_count = reply_effects
        .iter()
        .filter(|e| matches!(e, EffectKind::Http(_)))
        .count();
    assert_eq!(
        http_count, 3,
        "scan reply after Connected should emit 3 Http sync immediately, got: {:?}",
        reply_effects
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
    let cursor = h
        .module
        .cursor_for(helix_im::state::ChannelId::from_test_u64(42));
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
    h.module
        .register_channel(helix_im::state::ChannelId::from_test_u64(1), 200);

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
    let cursor = h
        .module
        .cursor_for(helix_im::state::ChannelId::from_test_u64(1));
    assert_eq!(
        cursor,
        Some(200),
        "scan should not overwrite pre-registered channel cursor"
    );
}
