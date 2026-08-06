//! Golden fixtures 回放——拿**真 Go 实际下发的帧**做确定性回放断言。
//!
//! 回归锚: C1（post 帧）/ B2（increment）/ B1（运维帧忽略）—— golden fixtures 工具链。
//!
//! ## 为什么有这个文件（防 post/posted 同类回归）
//!
//! 本会话最大坑：parser 曾按**猜测**的 `"posted"` 写，真帧其实是 `"post"`。根因是手写帧
//! 凭猜，与现网 wire 漂移而 CI 测不出。Golden fixtures 把「测试用帧」从「手写猜测」升级为
//! 「现网真抓」——fixture 文件 = 真 Go 服务端 RAW WS 下发原文（带来源头注释），测试 = 回放断言。
//! 任何 parser 字段路径 / action 名漂移，回放断言立刻红。
//!
//! ## 帧来源
//!
//! - `post.json`                  : live 抓帧，action=="post" 新消息广播（event_seq=40114）
//! - `increment_channel.json`     : devloop 抓帧，action=="increment_channel"（lastEventSeq=151）
//! - `increment_channel_end.json` : devloop 抓帧，action=="increment_channel_end"（global end）
//! - `status_change_ignored.json` : devloop 抓帧，action=="status_change" 运维帧（须忽略）
//!
//! 补真帧的「链」（怎么持续喂入新真帧）见 `tests/fixtures/real_go_frames/README.md`。

mod common;
use common::*;

use helix_core::effect::{SqlValue, StorageOp};
use helix_core::tick::InboundBytes;
use helix_core::{Effect, Tick};
use helix_im::state::ChannelId;

// ─── fixture 加载 ────────────────────────────────────────────────────────────

const POST_JSON: &str = include_str!("fixtures/real_go_frames/post.json");
const INCREMENT_JSON: &str = include_str!("fixtures/real_go_frames/increment_channel.json");
const INCREMENT_END_JSON: &str = include_str!("fixtures/real_go_frames/increment_channel_end.json");
const STATUS_CHANGE_JSON: &str = include_str!("fixtures/real_go_frames/status_change_ignored.json");

/// 取 fixture 的 JSON 主体——剥掉 `//` 来源头注释行，拼回单个 JSON 帧。
fn fixture_body(raw: &str) -> Vec<u8> {
    raw.lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
        .into_bytes()
}

fn inbound(raw: &str) -> Tick {
    Tick::Inbound(InboundBytes(fixture_body(raw).into()))
}

// ─── 断言辅助：从原始 sink 取 effect 细节（EffectKind 不带字段）──────────────────

fn as_text(v: &SqlValue) -> Option<&str> {
    match v {
        SqlValue::Text(s) => Some(s.as_str()),
        _ => None,
    }
}

/// 收集所有 message 表 upsert 行（含原子写屏障）的某列 TEXT 值。
fn message_upsert_col(sink: &helix_core::EffectSink, col: &str) -> Vec<String> {
    let mut out = Vec::new();
    for e in sink.as_slice() {
        let ops = match e {
            Effect::PersistFire { ops } => ops,
            Effect::Persist { ops, .. } => ops,
            Effect::PersistAtomic { ops, .. } => ops,
            _ => continue,
        };
        for op in ops {
            if let StorageOp::BatchUpsert(spec) = op {
                if spec.table != "message" {
                    continue;
                }
                for row in &spec.rows {
                    if let Some(v) = row
                        .iter()
                        .find(|(k, _)| k == col)
                        .and_then(|(_, v)| as_text(v))
                    {
                        out.push(v.to_string());
                    }
                }
            }
        }
    }
    out
}

// ─── ① post.json：真新消息帧 → 落库 message 行 + cursor 推进 ─────────────────────

/// 真 Go `action=="post"` 帧（event_seq=cursor+1）先形成 MessageV3 原子写：
/// - message 行列 temporary_id / id / message / read_bits 全对齐真帧；
/// - matching PersistOk 后把 cursor 推到 40114；
/// - viewer 绝对态读回完成后才发布终态事件。
///
/// **这是 post/posted 回归的活护栏**：若 parser 把 action 名写错（如退回 "posted"），
/// classify_inbound 会判 Ignored → 零 effect → 本测试立刻红。
#[test]
fn real_go_post_frame_persists_message_and_advances_cursor() {
    // post.json 真帧 channelId = rmibkwkjwidhdx5xy8pyse6a3r，event_seq = 40114。
    let channel =
        ChannelId::from_str("rmibkwkjwidhdx5xy8pyse6a3r").expect("真帧 channelId 合法 26 字符");

    // 注册该 channel，cursor 种到 40113，使真帧 event_seq(40114) == cursor+1 → 立即 apply。
    let mut h = Harness::new(&[]);
    h.module.register_channel(channel, 40113);
    h.start(0);

    let effects = h.step(inbound(POST_JSON), 1000);

    // 连续帧必须先形成唯一原子写屏障，回执前不得发布终态。
    let persist_corr = effects.iter().find_map(|effect| match effect {
        EffectKind::PersistAtomic(corr) => Some(*corr),
        _ => None,
    });
    let Some(persist_corr) = persist_corr else {
        panic!("真 post 帧必须形成 MessageV3 PersistAtomic，got {effects:?}");
    };
    assert!(
        !effects.iter().any(|e| *e == EffectKind::Emit),
        "真 post 帧持久回执前不得 emit，got {:?}",
        effects
    );

    // 落库 message 行字段全对齐真帧（防字段路径漂移：camelCase temporaryId/readBits 等）。
    assert_eq!(
        message_upsert_col(&h.sink, "temporary_id"),
        vec!["helixverify1781671289".to_string()],
        "message 行 temporary_id 须取真帧 data.temporaryId（camelCase 路径）"
    );
    assert_eq!(
        message_upsert_col(&h.sink, "id"),
        vec!["hjh19t4tutnt5ygxn4oxsqtbkc".to_string()],
        "message 行 id 须取真帧 server 权威 data.id"
    );
    assert_eq!(
        message_upsert_col(&h.sink, "message"),
        vec!["helix-verify-1781671289".to_string()],
        "message 行 message 内容须取真帧 data.message"
    );
    assert_eq!(
        message_upsert_col(&h.sink, "read_bits"),
        vec!["100".to_string()],
        "message 行 read_bits 须取真帧 data.readBits（camelCase 路径，防 c2 readBits 回归）"
    );

    // matching PersistOk 后推进 cursor，并发起 viewer channel 绝对态读回。
    let committed = h.step(port_reply_ok(persist_corr), 1001);
    assert!(
        committed
            .iter()
            .any(|effect| matches!(effect, EffectKind::Persist(_))),
        "真 post 帧提交后必须读回 viewer channel 绝对态"
    );
    assert_eq!(
        h.module.cursor_for(channel),
        Some(40114),
        "真 post 帧须把 cursor 推进到 data.event_seq=40114"
    );
}

/// 边界（Round-3 UC-4.5 偏差修复）：未注册 channel 收到真 post 帧 → **自动注册**（cursor=0 seed），
/// **不再静默丢弃**。对齐真源 `on_ws_event` 的 `gate_states.entry().or_insert_with` + `handle_new_post`
/// 无条件落库。post.json 真帧 event_seq=40114 > 新注册 cursor+1(=1) → 缺口：自动注册 + arm 1s gate
/// （sync 回拉前置消息）。首次见到的新群 / 被拉入群的首条在线 push 不丢。
#[test]
fn real_go_post_frame_on_unregistered_channel_auto_registers_and_arms_gate() {
    let channel =
        ChannelId::from_str("rmibkwkjwidhdx5xy8pyse6a3r").expect("真帧 channelId 合法 26 字符");
    let mut h = Harness::new(&[]);
    h.start(0);
    assert_eq!(h.module.cursor_for(channel), None, "前置：channel 未注册");

    let effects = h.step(inbound(POST_JSON), 1000);

    // 自动注册（cursor=0，未推进——缺口 40114 未填）：不再整帧丢弃。
    assert_eq!(
        h.module.cursor_for(channel),
        Some(0),
        "陌生 channel 真 post 帧应自动注册（cursor=0 seed），不丢弃，got {:?}",
        effects
    );
    // event_seq(40114) > cursor+1(1) → 缺口：arm 1s gate（sync 回拉前置消息）。
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, EffectKind::ScheduleTimer(_))),
        "陌生 channel 缺口 post 应 arm gate（缺口回拉），got {:?}",
        effects
    );
}

// ─── ② increment_channel.json：真增量帧 → 注册 + cursor 种子 + commit 后 emit ─────────

/// 真 Go `increment_channel` 帧 → 注册全新 channel + 冷启动 cursor 种子 0；global end
/// 形成相关 Persist，成功回执后才 emit。
#[test]
fn real_go_increment_channel_registers_and_seeds_cursor() {
    // increment_channel.json 真帧 data.id = bi643wys7fgy9pbai9wjfusnfa，lastEventSeq = 151。
    let channel = ChannelId::from_str("bi643wys7fgy9pbai9wjfusnfa").expect("真帧 id 合法 26 字符");

    let mut h = Harness::new_with_identity(
        &[],
        "444",
        "64118eebd2b665246b7880eb",
        "破坏者",
        "战士",
        "狂派事业部",
    );
    h.start(0);

    let effects = h.step(inbound(INCREMENT_JSON), 1000);

    // 修 todo#1：冷启动真新 channel 内存 cursor 种子 = 0（本地确认水位），**不是** data.lastEventSeq(151,
    // 服务端水位)。误种水位 → from_seq=max → Go no_change → 离线 backfill 永不触发。
    assert_eq!(
        h.module.cursor_for(channel),
        Some(0),
        "真 increment 帧冷启动 cursor 种子须为 0，不得用 data.lastEventSeq（服务端水位）"
    );
    // 记入 increment_fetched 批次集（needSync=true）。
    assert!(
        h.module.increment_fetched_contains(channel),
        "真 increment 帧须记入 increment_fetched 集"
    );
    // G-S1：逐帧只缓冲，持久化 ack 前不得 emit im:channel:increment。
    assert!(
        !effects.iter().any(|e| *e == EffectKind::Emit),
        "真 increment 帧持久化 ack 前不得 emit，got {:?}",
        effects
    );

    // increment_channel_end（global，data={}）→ 对增量群（含本群）发 sync Http。
    let end_effects = h.step(inbound(INCREMENT_END_JSON), 1100);
    let http = end_effects
        .iter()
        .filter(|e| matches!(e, EffectKind::Http(_)))
        .count();
    assert!(
        http >= 1,
        "真 increment_channel_end(global) 须对增量群发 sync Http，got {:?}",
        end_effects
    );
    // sync 发的是 /channel/sync/notify。
    let saw_sync_notify = h.sink.as_slice().iter().any(
        |e| matches!(e, Effect::Http { req, .. } if req.url.ends_with("/channel/sync/notify")),
    );
    assert!(
        saw_sync_notify,
        "真 increment_channel_end 须触发 /channel/sync/notify"
    );
    let persist_corr = end_effects
        .iter()
        .find_map(|effect| match effect {
            EffectKind::Persist(corr) => Some(*corr),
            _ => None,
        })
        .expect("真 increment_channel_end 必须形成相关 Persist");
    let committed = h.step(port_reply_ok(persist_corr), 1_200);
    assert_eq!(
        committed
            .iter()
            .filter(|effect| **effect == EffectKind::Emit)
            .count(),
        1,
        "事务成功后只发布 typed channel-sync-ready"
    );
    let emitted = h
        .sink
        .as_slice()
        .iter()
        .filter_map(|effect| match effect {
            Effect::Emit { event } => Some(String::from_utf8_lossy(event.0.as_ref()).to_string()),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert!(
        emitted
            .iter()
            .any(|body| body.contains("im:channel-sync-ready")),
        "Persist Ok 后必须发布 typed ready，got {:?}",
        emitted
    );
    assert!(
        emitted
            .iter()
            .all(|body| !body.contains("im:channel:increment")
                && !body.contains("im:channels:loaded")),
        "Persist Ok 后不得恢复 raw channel sync 事件，got {:?}",
        emitted
    );
}

// ─── ③ status_change.json：真运维帧 → 优雅忽略（零 effect / 零 panic）────────────

/// 真 Go `status_change` 运维帧 → InboundEvent::Ignored：零 effect、零 panic、零状态副作用。
#[test]
fn real_go_status_change_frame_is_ignored() {
    let mut h = Harness::new(&[]);
    h.start(0);

    let before_cursor = h.module.cursor_for(channel_id(1));

    let effects = h.step(inbound(STATUS_CHANGE_JSON), 1000);

    assert!(
        effects.is_empty(),
        "真 status_change 运维帧须优雅忽略（零 effect），got {:?}",
        effects
    );
    // 无任何 channel 副作用（cursor 不变）。
    assert_eq!(
        h.module.cursor_for(channel_id(1)),
        before_cursor,
        "运维帧不得改动任何 channel 状态"
    );
}

// ─── ④ fixture 健康检查：每个 fixture 都是合法 JSON 帧（剥头注释后）────────────────

/// 全部 fixture 主体都能被 serde_json 解析——防有人手改 fixture 改坏 JSON。
#[test]
fn all_fixtures_are_valid_json_after_stripping_header() {
    for (name, raw) in [
        ("post.json", POST_JSON),
        ("increment_channel.json", INCREMENT_JSON),
        ("increment_channel_end.json", INCREMENT_END_JSON),
        ("status_change_ignored.json", STATUS_CHANGE_JSON),
    ] {
        let body = fixture_body(raw);
        let parsed: Result<serde_json::Value, _> = serde_json::from_slice(&body);
        assert!(
            parsed.is_ok(),
            "fixture {} 剥头注释后须是合法 JSON：{:?}",
            name,
            parsed.err()
        );
        // 每个真帧都带顶层 action（真 Go 信封 {v,action,data,broadcast,seq}）。
        let v = parsed.unwrap();
        assert!(
            v.get("action").and_then(|a| a.as_str()).is_some(),
            "fixture {} 须含顶层 action（真 Go 信封）",
            name
        );
    }
}
