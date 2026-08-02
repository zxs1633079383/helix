//! P6 投影/状态查询命令集成测试（回放剧本）。
//!
//! 回归锚: P6/A2 —— `im_query_messages_by_channel`（Scan→coverage→必要时 HTTP）+
//! `im_delete_all_dialogs`
//! （清内存 channel gate + emit 刷新）端到端走 ImModule::handle。
//!
//! 单元级断言（build/emit 纯函数）在 `src/query.rs` 的 `#[cfg(test)]`；本文件验证
//! **dispatch 接线**：Tick::Command → handle → 正确 Effect 序列 + PortReply 回报 → emit 投影。

mod common;
use common::*;

use bytes::Bytes;
use helix_core::effect::{Effect, ScanOrder, ScanSpec, SqlValue, StorageOp};
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
        order_by,
    } = scan;
    assert_eq!(table, "message");
    assert_eq!(limit, Some(26));
    assert!(matches!(
        filter,
        Some(("channel_id", SqlValue::Text(s))) if s == cid(1)
    ));
    assert_eq!(
        order_by,
        &[
            ScanOrder::desc("create_at"),
            ScanOrder::desc("temporary_id"),
        ]
    );
}

/// 没有显式远端窗口 proof 时，即使本地非空也只能判 Partial，必须走远端。
#[test]
fn query_messages_reply_without_coverage_falls_back_to_remote() {
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

    assert!(matches!(
        h.effect_log.last().unwrap().as_slice(),
        [EffectKind::Http(_)]
    ));
    let (url, body) = h
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Http { req, .. } => Some((
                req.url.as_str(),
                serde_json::from_slice::<serde_json::Value>(req.body.as_ref()?.as_ref()).ok()?,
            )),
            _ => None,
        })
        .expect("Partial 应由 helix-im 发 getLatestPost");
    assert!(url.ends_with("/posts/getLatestPost"));
    assert_eq!(body["channelId"], cid(2));
    assert!(h
        .sink
        .as_slice()
        .iter()
        .all(|effect| !matches!(effect, Effect::Emit { .. })));
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

/// G10d：频道首段顺序与共享字段保持不变，第二段按当前认证用户合并 member top。
#[test]
fn dialog_list_merges_auth_scoped_member_top_after_channel_scan() {
    let mut h = Harness::new_with_identity(&[1, 2], "user-a", "", "", "", "");
    h.step(
        query_cmd(
            "im_query_dialog_list",
            serde_json::json!({ "limit": 2, "req_id": "dialog-g10d" }),
        ),
        1_000,
    );
    let first_corr = h
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Persist { corr, .. } => Some(corr.raw()),
            _ => None,
        })
        .expect("dialog query 应注册首段 channel Scan");
    let channel_rows = serde_json::json!([
        {
            "id": cid(1),
            "is_top": 1,
            "unread_count": 7,
            "last_post_at": 200,
            "created_at": 100
        },
        {
            "id": cid(2),
            "is_top": 0,
            "unread_count": 3,
            "last_post_at": 150,
            "created_at": 90
        }
    ]);
    h.step(
        port_reply_with_body(first_corr, Bytes::from(channel_rows.to_string())),
        1_001,
    );
    let member_corr = h
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Persist { corr, ops } => ops.iter().find_map(|op| match op {
                StorageOp::Scan(spec) => {
                    assert_eq!(spec.table, "channel_member");
                    assert_eq!(spec.limit, None);
                    assert_eq!(spec.order_by, &[]);
                    assert!(matches!(
                        spec.filter,
                        Some(("user_id", SqlValue::Text(ref user))) if user == "user-a"
                    ));
                    Some(corr.raw())
                }
                _ => None,
            }),
            _ => None,
        })
        .expect("首段成功后应以 auth_user_id 发起第二段 member Scan");
    let member_rows = serde_json::json!([
        {
            "channel_id": cid(1),
            "user_id": "user-a",
            "channel_is_top": 1
        },
        {
            "channel_id": cid(2),
            "user_id": "user-a",
            "channel_is_top": 0
        }
    ]);
    h.step(
        port_reply_with_body(member_corr, Bytes::from(member_rows.to_string())),
        1_002,
    );

    let list = h
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Emit { event } => {
                let value: serde_json::Value = serde_json::from_slice(event.0.as_ref()).ok()?;
                (value["event"] == "im:channel:list").then_some(value["data"]["channels"].clone())
            }
            _ => None,
        })
        .expect("第二段成功后应发布 channel list");
    assert_eq!(list[0]["id"], cid(1));
    assert_eq!(list[0]["channelIsTop"], true);
    assert_eq!(list[0]["is_top"], 1);
    assert_eq!(list[0]["unread_count"], 7);
    assert_eq!(list[1]["id"], cid(2));
    assert_eq!(list[1]["channelIsTop"], false);
    assert_eq!(list[1]["is_top"], 0);
    assert_eq!(list[1]["unread_count"], 3);
}

/// G10d：第二段若携带其他用户或畸形关联，必须失败且不发布带泄漏值的成功快照。
#[test]
fn dialog_list_rejects_member_scope_leak_or_missing_row() {
    let mut h = Harness::new_with_identity(&[1], "user-a", "", "", "", "");
    h.step(
        query_cmd("im_query_dialog_list", serde_json::json!({})),
        1_000,
    );
    let first_corr = h
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Persist { corr, .. } => Some(corr.raw()),
            _ => None,
        })
        .expect("首段 corr");
    h.step(
        port_reply_with_body(
            first_corr,
            Bytes::from(serde_json::json!([{ "id": cid(1), "is_top": 0 }]).to_string()),
        ),
        1_001,
    );
    let member_corr = h
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Persist { corr, .. } => Some(corr.raw()),
            _ => None,
        })
        .expect("第二段 corr");
    h.step(
        port_reply_with_body(
            member_corr,
            Bytes::from(
                serde_json::json!([
                    { "channel_id": cid(1), "user_id": "user-b", "channel_is_top": 1 }
                ])
                .to_string(),
            ),
        ),
        1_002,
    );
    let failed = h
        .sink
        .as_slice()
        .iter()
        .filter_map(|effect| match effect {
            Effect::Emit { event } => {
                serde_json::from_slice::<serde_json::Value>(event.0.as_ref()).ok()
            }
            _ => None,
        })
        .find(|value| value["event"] == "im:channel:list")
        .expect("scope 泄漏应回传明确 failed list");
    assert_eq!(failed["data"]["state"], "failed");
    assert!(h.sink.as_slice().iter().all(|effect| {
        !matches!(effect, Effect::Emit { event } if serde_json::from_slice::<serde_json::Value>(event.0.as_ref())
            .ok()
            .is_some_and(|value| value["event"] == "im:channel:list" && value["data"].get("channels").is_some_and(|channels| channels.as_array().is_some_and(|channels| !channels.is_empty()))))
    }));
}

/// G10d：缺失首段频道对应的 member 行时 fail-closed，避免从 shared channel.is_top 猜测。
#[test]
fn dialog_list_rejects_missing_member_row_without_shared_top_fallback() {
    let mut h = Harness::new_with_identity(&[1, 2], "user-a", "", "", "", "");
    h.step(
        query_cmd("im_query_dialog_list", serde_json::json!({})),
        1_000,
    );
    let first_corr = h
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Persist { corr, .. } => Some(corr.raw()),
            _ => None,
        })
        .expect("首段 corr");
    h.step(
        port_reply_with_body(
            first_corr,
            Bytes::from(
                serde_json::json!([
                    { "id": cid(1), "is_top": 1 },
                    { "id": cid(2), "is_top": 0 }
                ])
                .to_string(),
            ),
        ),
        1_001,
    );
    let member_corr = h
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Persist { corr, .. } => Some(corr.raw()),
            _ => None,
        })
        .expect("第二段 corr");
    h.step(
        port_reply_with_body(
            member_corr,
            Bytes::from(
                serde_json::json!([
                    { "channel_id": cid(1), "user_id": "user-a", "channel_is_top": 1 }
                ])
                .to_string(),
            ),
        ),
        1_002,
    );
    let failed = h
        .sink
        .as_slice()
        .iter()
        .filter_map(|effect| match effect {
            Effect::Emit { event } => {
                serde_json::from_slice::<serde_json::Value>(event.0.as_ref()).ok()
            }
            _ => None,
        })
        .find(|value| value["event"] == "im:channel:list")
        .expect("缺失 member 行应 fail-closed");
    assert_eq!(failed["data"]["state"], "failed");
}
