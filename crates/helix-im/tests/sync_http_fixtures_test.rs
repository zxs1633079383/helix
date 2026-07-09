//! Sync HTTP-response golden fixtures 回放——锁 `/channel/sync/notify` reply body 的真 wire 形态。
//!
//! 回归锚: P0（基线锁定 / golden fixtures 工具链）。
//!
//! ## 为什么有这个文件
//!
//! `real_go_frames/README.md`「已知缺口」记录：sync 的 `no_change`/`events`/`too_long` 走 **HTTP**
//! （`/channel/sync/notify` 的 reply body），不在 WS RAW 帧 dump 里——需 driver HTTP 层 dump 才能抓
//! （该 dump 为 P1+ 接力项，见 README「已知缺口」）。本批 P0 先落三个**占位** fixture
//! （PLACEHOLDER · 待真抓，头注释手填 parser 文档 wire 形态）。
//!
//! 本测试把占位 fixture 喂**真 parser**（`parse_sync_response`，与现网解析同一份代码），断言：
//! ① 剥头注释后是合法 JSON；② parse 出文档承诺的 `SyncResponse` 变体 + 关键字段。
//! 真帧抓到覆盖占位文件后，本测试**无需改动**即继续守护 wire 形态——这就是 fixtures「链」。

use helix_im::parser::parse_sync_response;
use helix_im::state::ChannelId;
use helix_im::sync_session::SyncResponse;

const SYNC_NO_CHANGE: &str = include_str!("fixtures/real_go_frames/sync_no_change.json");
const SYNC_EVENTS: &str = include_str!("fixtures/real_go_frames/sync_events.json");
const SYNC_TOO_LONG: &str = include_str!("fixtures/real_go_frames/sync_too_long.json");

/// 真帧 channelId（三个 sync fixture 共用，与 post.json 同 channel）。
const FIXTURE_CHANNEL: &str = "rmibkwkjwidhdx5xy8pyse6a3r";

/// 剥 `//` 来源/占位头注释，拼回 JSON 主体（与 golden_replay_test::fixture_body 同约定）。
fn fixture_body(raw: &str) -> Vec<u8> {
    raw.lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
        .into_bytes()
}

fn fixture_channel() -> ChannelId {
    ChannelId::from_str(FIXTURE_CHANNEL).expect("fixture channelId 合法 26 字符")
}

#[test]
fn all_sync_fixtures_are_valid_json_after_stripping_header() {
    for (name, raw) in [
        ("sync_no_change", SYNC_NO_CHANGE),
        ("sync_events", SYNC_EVENTS),
        ("sync_too_long", SYNC_TOO_LONG),
    ] {
        let body = fixture_body(raw);
        let parsed: serde_json::Value = serde_json::from_slice(&body)
            .unwrap_or_else(|e| panic!("{name} 剥头注释后必为合法 JSON: {e}"));
        assert_eq!(
            parsed["status"], "SUCCESS",
            "{name} 应为 SUCCESS 信封（parser 零信任前置）"
        );
    }
}

#[test]
fn no_change_fixture_parses_to_no_change() {
    let body = fixture_body(SYNC_NO_CHANGE);
    let resp = parse_sync_response(&body, fixture_channel()).expect("no_change 应可解析");
    assert!(
        matches!(resp, SyncResponse::NoChange),
        "kind=no_change 应解析为 SyncResponse::NoChange，实得 {resp:?}"
    );
}

#[test]
fn events_fixture_parses_to_events_with_authoritative_next_seq() {
    let body = fixture_body(SYNC_EVENTS);
    let resp = parse_sync_response(&body, fixture_channel()).expect("events 应可解析");
    match resp {
        SyncResponse::Events {
            events,
            next_seq,
            needs_continuation,
            ..
        } => {
            assert_eq!(events.len(), 2, "占位 events 含 2 条");
            // cursor 锚服务端权威 nextSeq（HX-C008），占位 nextSeq=40114=末条 seq。
            assert_eq!(next_seq.0, 40114, "cursor 锚 nextSeq 非 max 推导");
            assert!(
                !needs_continuation,
                "2 条 < 满批 500，不应触发续拉（needs_continuation 由满批派生，无显式标志）"
            );
        }
        other => panic!("kind=events 应解析为 SyncResponse::Events，实得 {other:?}"),
    }
}

#[test]
fn too_long_fixture_parses_to_too_long_with_reset_to() {
    let body = fixture_body(SYNC_TOO_LONG);
    let resp = parse_sync_response(&body, fixture_channel()).expect("too_long 应可解析");
    match resp {
        SyncResponse::TooLong { reset_to } => {
            assert_eq!(
                reset_to.0, 40114,
                "too_long 的 resetTo 锚 maxSeq（后续应用层 drop 本地消息并重拉）"
            );
        }
        other => panic!("kind=too_long 应解析为 SyncResponse::TooLong，实得 {other:?}"),
    }
}
