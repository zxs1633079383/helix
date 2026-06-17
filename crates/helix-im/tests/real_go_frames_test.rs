//! B1：真 Go 信封帧分类回放——hello 握手 + 运维帧优雅忽略。
//!
//! 用真 Go 抓取的 fixtures（tests/fixtures/real_go_frames/connect_init.jsonl）验证：
//! - 真 hello 帧（顶层 action+data.connectionId）→ emit im:connection:established。
//! - 真运维帧（status_change / multiple_channels_viewed）→ registry ignored handler → 零 effect、零 panic。
//!
//! 回归锚: B1（真 Go wire 信封 {v,action,data,broadcast,seq}，action 在顶层）

mod common;
use common::*;

use helix_core::effect::Effect;
use helix_core::tick::InboundBytes;
use helix_core::CoreError;
use helix_core::Module;
use helix_core::Tick;
use helix_im::state::ChannelId;
use helix_im::ImError;

const REAL_FRAMES: &str = include_str!("fixtures/real_go_frames/connect_init.jsonl");

fn inbound_raw(line: &str) -> Tick {
    Tick::Inbound(InboundBytes(line.as_bytes().to_vec().into()))
}

fn inbound_json(value: serde_json::Value) -> Tick {
    Tick::Inbound(InboundBytes(serde_json::to_vec(&value).unwrap().into()))
}

fn frame_lines() -> Vec<String> {
    REAL_FRAMES
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect()
}

#[test]
fn real_go_hello_completes_handshake_and_emits_connection_established() {
    let lines = frame_lines();
    assert!(lines.len() >= 3, "fixtures 至少含 hello + 2 运维帧");

    let mut h = Harness::new(&[]);
    h.start(0);

    // line 0 = 真 hello 帧
    let _ = h.step(inbound_raw(&lines[0]), 100);
    assert!(
        h.sink.as_slice().iter().any(|effect| matches!(
            effect,
            Effect::Emit { event }
                if String::from_utf8_lossy(event.0.as_ref())
                    .contains("im:connection:established")
        )),
        "真 hello 帧应 emit im:connection:established，got {:?}",
        h.sink.as_slice()
    );
}

#[test]
fn real_go_operational_frames_ignored_no_effect_no_panic() {
    let lines = frame_lines();

    let mut h = Harness::new(&[]);
    h.start(0);
    // 先 hello 完成握手
    h.step(inbound_raw(&lines[0]), 100);

    // line 1+ = 运维帧（status_change / multiple_channels_viewed）→ 优雅忽略，零 effect
    for line in &lines[1..] {
        let effects = h.step(inbound_raw(line), 200);
        assert!(
            effects.is_empty(),
            "运维帧应零 effect 优雅忽略，frame={}，got {:?}",
            line,
            effects
        );
    }
}

#[test]
fn production_inbound_unknown_action_returns_unsupported_ws_action() {
    let mut h = Harness::new(&[]);
    let tick = inbound_json(serde_json::json!({
        "v": 1,
        "action": "future_unknown_action",
        "data": {}
    }));

    let err = h
        .module
        .handle(&tick, 100, &mut h.sink)
        .expect_err("unknown top-level action should fail through production path");

    match err {
        CoreError::ModuleError { module, source } => {
            assert_eq!(module, "helix-im");
            let im_err = source
                .downcast_ref::<ImError>()
                .expect("source should be ImError");
            assert!(
                matches!(im_err, ImError::UnsupportedWsAction(action) if action == "future_unknown_action"),
                "expected UnsupportedWsAction(future_unknown_action), got {im_err:?}"
            );
        }
        other => panic!("expected ModuleError, got {other:?}"),
    }
    assert!(
        h.sink.as_slice().is_empty(),
        "unsupported action should not produce effects before returning error"
    );
}

#[test]
fn production_inbound_invalid_json_is_ignored_without_effects() {
    let mut h = Harness::new(&[1]);
    let tick = Tick::Inbound(InboundBytes(bytes::Bytes::from_static(b"{")));

    h.module
        .handle(&tick, 100, &mut h.sink)
        .expect("invalid JSON inbound should remain backward-compatible no-op");

    assert!(
        h.sink.as_slice().is_empty(),
        "invalid JSON should not produce effects"
    );
    assert_eq!(
        h.module.cursor_for(ChannelId::from_test_u64(1)),
        Some(0),
        "invalid JSON should not advance cursor"
    );
}

#[test]
fn production_inbound_explicit_ignored_actions_are_noop() {
    let mut h = Harness::new(&[1]);
    let channel_id = cid(1);

    for action in [
        "status_change",
        "multiple_channels_viewed",
        "typing",
        "presence",
    ] {
        h.sink.clear();
        let tick = inbound_json(serde_json::json!({
            "v": 1,
            "action": action,
            "data": { "channel_id": channel_id }
        }));

        h.module
            .handle(&tick, 100, &mut h.sink)
            .unwrap_or_else(|err| panic!("{action} should be ignored, got {err:?}"));

        assert!(
            h.sink.as_slice().is_empty(),
            "{action} should not produce effects"
        );
        assert_eq!(
            h.module.cursor_for(ChannelId::from_test_u64(1)),
            Some(0),
            "{action} should not advance cursor"
        );
        assert_eq!(
            h.module.pending_send_count(),
            0,
            "{action} should not touch pending sends"
        );
    }
}
