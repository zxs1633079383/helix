//! B1：真 Go 信封帧分类回放——hello 握手 + 运维帧优雅忽略。
//!
//! 用真 Go 抓取的 fixtures（tests/fixtures/real_go_frames/connect_init.jsonl）验证：
//! - 真 hello 帧（顶层 action+data.connectionId）→ emit im:connection:established。
//! - 真运维帧（status_change / multiple_channels_viewed）→ FrameClass::Ignored → 零 effect、零 panic。
//!
//! 回归锚: B1（真 Go wire 信封 {v,action,data,broadcast,seq}，action 在顶层）

mod common;
use common::*;

use helix_core::tick::InboundBytes;
use helix_core::Tick;

const REAL_FRAMES: &str = include_str!("fixtures/real_go_frames/connect_init.jsonl");

fn inbound_raw(line: &str) -> Tick {
    Tick::Inbound(InboundBytes(line.as_bytes().to_vec().into()))
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
    let hello_effects = h.step(inbound_raw(&lines[0]), 100);
    assert!(
        hello_effects.iter().any(|e| matches!(e, EffectKind::Emit)),
        "真 hello 帧应 emit im:connection:established，got {:?}",
        hello_effects
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
