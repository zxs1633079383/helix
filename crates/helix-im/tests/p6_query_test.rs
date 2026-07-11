//! P6 投影/状态查询命令集成测试（回放剧本）。
//!
//! 回归锚: P6 —— `im_query_messages_by_channel`（Scan→emit 投影）+ `im_delete_all_dialogs`
//! （清内存 channel gate + emit 刷新）端到端走 ImModule::handle。
//!
//! 单元级断言（build/emit 纯函数）在 `src/query.rs` 的 `#[cfg(test)]`；本文件验证
//! **dispatch 接线**：Tick::Command → handle → 正确 Effect 序列 + PortReply 回报 → emit 投影。

mod common;
use common::*;

use bytes::Bytes;
use helix_core::effect::{Effect, ScanSpec, SqlValue, StorageOp};
use helix_core::tick::AppCommand;
use helix_core::Tick;

/// 构造任意 query 命令 Tick。
fn query_cmd(name: &'static str, payload: serde_json::Value) -> Tick {
    Tick::Command(AppCommand::new(name, serde_json::to_vec(&payload).unwrap()))
}

/// im_query_messages_by_channel → 吐 Persist{Scan(message, filter=channel_id, order=create_at DESC)}。
#[test]
fn query_messages_dispatches_filtered_scan() {
    let mut h = Harness::new(&[1]);
    let kinds = h.step(
        query_cmd(
            "im_query_messages_by_channel",
            serde_json::json!({ "channel_id": cid(1), "limit": 25 }),
        ),
        1_000,
    );
    // 恰一条 Persist（Scan）。
    assert_eq!(kinds.len(), 1, "应吐恰一条 Effect（Scan）");
    let scan = h
        .sink
        .as_slice()
        .iter()
        .find_map(|e| match e {
            Effect::Persist { ops, .. } => ops.iter().find_map(|op| match op {
                StorageOp::Scan(s) => Some(s.clone()),
                _ => None,
            }),
            _ => None,
        })
        .expect("应有 Scan op");
    let ScanSpec {
        table,
        limit,
        filter,
        order_desc_col,
    } = scan;
    assert_eq!(table, "message");
    assert_eq!(limit, Some(25));
    assert!(matches!(
        filter,
        Some(("channel_id", SqlValue::Text(s))) if s == cid(1)
    ));
    assert_eq!(order_desc_col, Some("create_at"));
}

/// Scan 回报（PortReply Ok with rows）→ emit im:messages:query_result 投影（含 messages 数组）。
#[test]
fn query_messages_reply_emits_projection() {
    let mut h = Harness::new(&[2]);
    // 触发查询 → 拿到注册的 corr。
    h.step(
        query_cmd(
            "im_query_messages_by_channel",
            serde_json::json!({ "channel_id": cid(2) }),
        ),
        1_000,
    );
    let corr = h
        .sink
        .as_slice()
        .iter()
        .find_map(|e| match e {
            Effect::Persist { corr, .. } => Some(corr.raw()),
            _ => None,
        })
        .expect("query 应注册 Persist corr");

    // 回报 rows（engine rows→JSON 数组约定）。
    let rows = Bytes::from(
        serde_json::to_vec(&serde_json::json!([
            { "temporary_id": "t1", "message": "hi", "create_at": 200 },
            { "temporary_id": "t2", "message": "yo", "create_at": 100 }
        ]))
        .unwrap(),
    );
    h.step(port_reply_with_body(corr, rows), 1_100);

    // 末批应含一条 Emit，载荷为 im:messages:query_result 投影。
    let emit = h
        .sink
        .as_slice()
        .iter()
        .find_map(|e| match e {
            Effect::Emit { event } => Some(event.0.clone()),
            _ => None,
        })
        .expect("回报后应 emit 投影");
    let v: serde_json::Value = serde_json::from_slice(emit.as_ref()).unwrap();
    assert_eq!(v["event"], "im:messages:query_result");
    assert_eq!(v["data"]["channel_id"], cid(2));
    // Scan 以 DESC 截取最近 N 条，Helix 投影反转成 ASC；前端只绑定 render-ready 行。
    assert_eq!(v["data"]["messages"][0]["temporaryId"], "t2");
    assert_eq!(v["data"]["messages"][0]["createAt"], 100);
    assert_eq!(v["data"]["messages"][1]["temporaryId"], "t1");
    assert_eq!(v["data"]["messages"][1]["createAt"], 200);
}

/// im_delete_all_dialogs → 清内存 channel gate（channels 空）+ emit im:channels:loaded。
#[test]
fn delete_all_dialogs_clears_channels_and_emits_refresh() {
    let mut h = Harness::new(&[1, 2, 3]);
    let kinds = h.step(
        query_cmd("im_delete_all_dialogs", serde_json::json!({})),
        1_000,
    );
    // 恰一条 Emit（刷新信号）。
    assert_eq!(kinds, vec![EffectKind::Emit]);
    let emit = h
        .sink
        .as_slice()
        .iter()
        .find_map(|e| match e {
            Effect::Emit { event } => Some(event.0.clone()),
            _ => None,
        })
        .expect("应 emit");
    let v: serde_json::Value = serde_json::from_slice(emit.as_ref()).unwrap();
    assert_eq!(v["event"], "im:channels:loaded");
    assert_eq!(v["data"]["reason"], "delete_all_dialogs");
}

/// 缺 channel_id → handle 返回 Err（dispatch 把 ImError 包成 CoreError；harness.step unwrap 会 panic，
/// 故这里直接调 module.handle 验证 Err，不走 step）。
#[test]
fn query_messages_missing_channel_id_errors() {
    use helix_core::{EffectSink, Module};
    let mut h = Harness::new(&[1]);
    let mut sink = EffectSink::new();
    let tick = query_cmd(
        "im_query_messages_by_channel",
        serde_json::json!({ "limit": 5 }),
    );
    let res = h.module.handle(&tick, 1_000, &mut sink);
    assert!(res.is_err(), "缺 channel_id 应返回 CoreError，不 panic");
}

/// 未知非 query 命令 → no-op（debug log），不产 Effect、不 panic。
#[test]
fn unknown_command_is_noop() {
    let mut h = Harness::new(&[1]);
    let kinds = h.step(
        query_cmd("im_totally_unknown", serde_json::json!({})),
        1_000,
    );
    assert!(kinds.is_empty(), "未知命令应 no-op 无 Effect");
}
