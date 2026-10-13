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

/// Dialog list 保持频道分页与排序，在成员回读后发布当前 viewer 的唯一 typed Result。
#[test]
fn dialog_list_merges_auth_scoped_member_top_after_channel_scan() {
    let mut h = Harness::new_with_identity(&[1, 2], "user-a", "company-a", "", "", "");
    let channel_rows = serde_json::json!([
        {
            "id": cid(1),
            "team_id": "company-a",
            "user_id": "user-a",
            "is_top": 1,
            "unread_count": 7,
            "last_post_at": 200,
            "created_at": 100
        },
        {
            "id": cid(2),
            "team_id": "company-a",
            "user_id": "user-a",
            "members": [{ "userId": "user-a" }],
            "is_top": 0,
            "unread_count": 3,
            "last_post_at": 150,
            "created_at": 90
        }
    ]);
    let member_corr = dialog_absolute_member_corr(&mut h, "dialog-g10d", channel_rows);
    let mut first_member = dialog_absolute_member_row();
    first_member["unread_count"] = serde_json::json!(7);
    let mut second_member = dialog_absolute_member_row();
    second_member["channel_id"] = serde_json::json!(cid(2));
    second_member["channel_is_top"] = serde_json::json!(0);
    second_member["unread_count"] = serde_json::json!(3);
    h.step(
        port_reply_with_body(
            member_corr,
            Bytes::from(serde_json::json!([first_member, second_member]).to_string()),
        ),
        2_002,
    );
    let result = dialog_absolute_result(&h, "dialog-g10d");
    assert_eq!(result["data"]["req_id"], "dialog-g10d");
    let list = &result["data"]["body"]["items"];
    assert_eq!(list[0]["id"], cid(1));
    assert_eq!(list[0]["is_top"], 1);
    assert_eq!(list[0]["unread_count"], 7);
    assert_eq!(list[1]["id"], cid(2));
    assert_eq!(list[1]["is_top"], 0);
    assert_eq!(list[1]["unread_count"], 3);
    assert_eq!(
        h.sink.as_slice().len(),
        1,
        "成员回读后不得发第三段 Scan 或旧业务广播"
    );
}

/// 成员回读缺少当前 viewer 时必须剔除频道，不能恢复共享行的旧成功快照。
#[test]
fn dialog_list_rejects_member_scope_leak_or_missing_row() {
    let mut h = Harness::new_with_identity(&[1], "user-a", "company-a", "", "", "");
    let member_corr = dialog_absolute_member_corr(
        &mut h,
        "dialog-other-viewer",
        dialog_absolute_channel_rows(),
    );
    let mut member = dialog_absolute_member_row();
    member["user_id"] = serde_json::json!("user-b");
    h.step(
        port_reply_with_body(
            member_corr,
            Bytes::from(serde_json::json!([member]).to_string()),
        ),
        2_002,
    );
    let result = dialog_absolute_result(&h, "dialog-other-viewer");
    assert_eq!(result["data"]["body"]["items"], serde_json::json!([]));
}

/// 跨租户频道在成员 Scan 前剔除；缺少当前成员时不能从 shared top 猜测可见性。
#[test]
fn dialog_list_rejects_missing_member_row_without_shared_top_fallback() {
    let mut h = Harness::new_with_identity(&[1, 2], "user-a", "company-a", "", "", "");
    let member_corr = dialog_absolute_member_corr(
        &mut h,
        "dialog-visible-member",
        serde_json::json!([
            {
                "id": cid(1),
                "team_id": "company-a",
                "user_id": "user-a",
                "is_top": 1
            },
            {
                "id": cid(2),
                "team_id": "company-b",
                "user_id": "user-a",
                "is_top": 0
            }
        ]),
    );
    h.step(
        port_reply_with_body(
            member_corr,
            Bytes::from(serde_json::json!([dialog_absolute_member_row()]).to_string()),
        ),
        2_002,
    );
    let result = dialog_absolute_result(&h, "dialog-visible-member");
    let items = result["data"]["body"]["items"]
        .as_array()
        .expect("typed Result items must be an array");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["id"], cid(1));
}

/// 从公开命令入口回放频道首段，并要求成员权威读回前不得发布旧频道占位值。
fn dialog_absolute_member_corr(h: &mut Harness, req_id: &str, channels: serde_json::Value) -> u64 {
    let expected_scope = channels
        .as_array()
        .unwrap()
        .iter()
        .filter(|row| row["team_id"] == "company-a")
        .map(|row| row["id"].as_str().unwrap().to_string())
        .collect::<Vec<_>>();
    let payload = serde_json::to_vec(&serde_json::json!({ "limit": 2, "reqId": req_id })).unwrap();
    h.step(
        Tick::Command(
            helix_im::client_api::build_command("im_query_dialog_list", &payload).unwrap(),
        ),
        2_000,
    );
    let channel_corr = h
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Persist { corr, ops }
                if matches!(ops.as_slice(), [StorageOp::Scan(spec)] if spec.table == "channel") =>
            {
                let StorageOp::Scan(spec) = &ops[0] else {
                    unreachable!()
                };
                assert_eq!(spec.limit, Some(2));
                assert_eq!(
                    spec.order_by,
                    &[
                        ScanOrder::desc("is_top"),
                        ScanOrder::desc("last_post_at"),
                        ScanOrder::desc("created_at"),
                    ]
                );
                Some(corr.raw())
            }
            _ => None,
        })
        .expect("公开 dialog 查询必须先读取频道");
    h.step(
        port_reply_with_body(channel_corr, Bytes::from(channels.to_string())),
        2_001,
    );
    assert!(
        h.sink
            .as_slice()
            .iter()
            .all(|effect| !matches!(effect, Effect::Emit { .. })),
        "成员绝对态读回前不得返回 shared channel 中的未读占位值"
    );
    assert_eq!(h.sink.as_slice().len(), 1, "首段后只应发起成员读取");
    h.sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Persist { corr, ops } => match ops.as_slice() {
                [StorageOp::ScopedScan(spec)] if spec.table == "channel_member" => {
                    assert_eq!(spec.scope_col, "channel_id");
                    assert_eq!(spec.scope_values.len(), expected_scope.len());
                    for (value, expected_id) in spec.scope_values.iter().zip(&expected_scope) {
                        assert!(
                            matches!(value, SqlValue::Text(channel_id) if channel_id == expected_id)
                        );
                    }
                    Some(corr.raw())
                }
                _ => None,
            },
            _ => None,
        })
        .expect("成员读取必须复用按真实频道 ID 限定的 ScopedScan")
}

/// 构造同租户频道的过期共享快照，故意保留旧成员占位与未读值。
fn dialog_absolute_channel_rows() -> serde_json::Value {
    serde_json::json!([{
        "id": cid(1),
        "team_id": "company-a",
        "user_id": "user-a",
        "members": [{ "userId": "user-a" }],
        "is_top": 0,
        "unread_count": 99,
        "unread_post_id": "stale-anchor",
        "reaction_unread_count": 99,
        "last_read_seq": 1,
        "projection_revision": 1,
        "last_post": "{\"id\":\"shared-old\",\"message\":\"过期共享消息\"}",
        "last_post_at": 100,
        "mention_count": 99,
        "urgent_count": 99
    }])
}

/// 成员公司可与频道不同；当前 viewer 的持久绝对态才是 badge 与锚点权威。
fn dialog_absolute_member_row() -> serde_json::Value {
    serde_json::json!({
        "channel_id": cid(1),
        "user_id": "user-a",
        "team_id": "company-external",
        "role": "MEMBER",
        "channel_is_top": 1,
        "unread_count": 5,
        "unread_post_id": "member-anchor",
        "reaction_unread_count": 2,
        "last_read_seq": 42,
        "projection_revision": 43,
        "effect_id": "dialog-member-effect-43",
        "last_post": "{\"id\":\"member-last\",\"message\":\"成员最后消息\"}",
        "last_post_at": 300,
        "last_root_post_at": 299,
        "mention_count": 3,
        "mention_count_root": 2,
        "mention_list": "[\"mention-1\"]",
        "mention_user": "成员提醒人",
        "urgent_mention_count": 1,
        "urgent_post_list": "[\"urgent-1\"]",
        "urgent_mention_user": "成员加急人"
    })
}

/// 只接受公开 handle 产生的唯一 typed Result，禁止兼容频道/已读广播。
fn dialog_absolute_result(h: &Harness, req_id: &str) -> serde_json::Value {
    assert_eq!(h.sink.as_slice().len(), 1, "终态必须只有一个 typed Result");
    let Effect::Emit { event } = &h.sink.as_slice()[0] else {
        panic!("终态必须为 Emit")
    };
    let result: serde_json::Value = serde_json::from_slice(event.0.as_ref()).unwrap();
    assert_eq!(result["event"], "im:read:result");
    assert_eq!(result["data"]["req_id"], req_id);
    result
}

/// 回归锚 e64e4616：成员绝对态覆盖共享值，同时保留最新 reaction/read-seq 合同。
#[test]
fn dialog_query_absolute_member_state_replaces_stale_shared_values() {
    let mut h = Harness::new_with_identity(&[1], "user-a", "company-a", "", "", "");
    let corr =
        dialog_absolute_member_corr(&mut h, "dialog-absolute", dialog_absolute_channel_rows());
    let mut member = dialog_absolute_member_row();
    // 成员回包即使有 id，也不能被 channel owner observer 误记为外公司频道。
    member["id"] = serde_json::json!(cid(1));
    h.step(
        port_reply_with_body(corr, Bytes::from(serde_json::json!([member]).to_string())),
        2_002,
    );
    let result = dialog_absolute_result(&h, "dialog-absolute");
    let items = result["data"]["body"]["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    let item = &items[0];
    assert_eq!(item["id"], cid(1));
    assert_eq!(item["teamId"], "company-a");
    assert_eq!(item["channelIsTop"], true);
    assert_eq!(item["is_top"], 1);
    assert_eq!(item["unreadCount"], 5);
    assert_eq!(item["unread_count"], 5);
    assert_eq!(item["unreadPostId"], "member-anchor");
    assert_eq!(item["reactionUnreadCount"], 2);
    assert_eq!(item["lastReadSeq"], 42);
    assert_eq!(item["projectionRevision"], 43);
    assert_eq!(item["effectId"], "dialog-member-effect-43");
    assert_eq!(item["lastPost"]["id"], "member-last");
    assert_eq!(item["lastPostAt"], 300);
    assert_eq!(item["lastRootPostAt"], 299);
    assert_eq!(item["mentionCount"], 3);
    assert_eq!(item["mentionCountRoot"], 2);
    assert_eq!(item["mentionList"], serde_json::json!(["mention-1"]));
    assert_eq!(item["urgentCount"], 1);
    assert_eq!(item["urgentPostList"], serde_json::json!(["urgent-1"]));
}

/// 最新成员合同：零未读清空旧锚点，没有 effect identity 的缓存不得宣称 revision。
#[test]
fn dialog_query_absolute_zero_clears_anchor_and_unproven_revision() {
    let mut h = Harness::new_with_identity(&[1], "user-a", "company-a", "", "", "");
    let corr = dialog_absolute_member_corr(&mut h, "dialog-read", dialog_absolute_channel_rows());
    let mut member = dialog_absolute_member_row();
    member["unread_count"] = serde_json::json!(0);
    member["unread_post_id"] = serde_json::json!("stale-member-anchor");
    member.as_object_mut().unwrap().remove("effect_id");
    h.step(
        port_reply_with_body(corr, Bytes::from(serde_json::json!([member]).to_string())),
        2_002,
    );
    let result = dialog_absolute_result(&h, "dialog-read");
    let item = &result["data"]["body"]["items"][0];
    assert_eq!(item["unreadCount"], 0);
    assert_eq!(item["unreadPostId"], "");
    assert_eq!(item["lastReadSeq"], 42);
    assert!(item.get("projectionRevision").is_none());
    assert!(item.get("projection_revision").is_none());
}

/// 缺少当前 viewer 或畸形成员权威必须 fail-closed，不能回退到频道 members 占位。
#[test]
fn dialog_query_absolute_missing_or_malformed_member_fails_closed() {
    let mut wrong_viewer = dialog_absolute_member_row();
    wrong_viewer["user_id"] = serde_json::json!("user-b");
    let mut missing_anchor = dialog_absolute_member_row();
    missing_anchor
        .as_object_mut()
        .unwrap()
        .remove("unread_post_id");
    let mut negative_reaction = dialog_absolute_member_row();
    negative_reaction["reaction_unread_count"] = serde_json::json!(-1);
    let mut missing_read_seq = dialog_absolute_member_row();
    missing_read_seq
        .as_object_mut()
        .unwrap()
        .remove("last_read_seq");
    for members in [
        serde_json::json!([]),
        serde_json::json!([wrong_viewer]),
        serde_json::json!([missing_anchor]),
        serde_json::json!([negative_reaction]),
        serde_json::json!([missing_read_seq]),
    ] {
        let mut h = Harness::new_with_identity(&[1], "user-a", "company-a", "", "", "");
        let corr =
            dialog_absolute_member_corr(&mut h, "dialog-invalid", dialog_absolute_channel_rows());
        h.step(
            port_reply_with_body(corr, Bytes::from(members.to_string())),
            2_002,
        );
        let result = dialog_absolute_result(&h, "dialog-invalid");
        assert_eq!(result["data"]["body"]["items"], serde_json::json!([]));
    }
}

/// 成员读失败结束 typed waiter，重复/迟到的相同 corr 不得再发布第二个终态。
#[test]
fn dialog_query_absolute_member_error_is_terminal_once() {
    use helix_core::tick::{PortError, PortOutcome};
    let mut h = Harness::new_with_identity(&[1], "user-a", "company-a", "", "", "");
    let corr = dialog_absolute_member_corr(&mut h, "dialog-error", dialog_absolute_channel_rows());
    h.step(
        Tick::PortReply {
            corr: helix_core::Correlation::from_raw(corr),
            outcome: PortOutcome::Err(PortError::Storage(500)),
        },
        2_002,
    );
    let result = dialog_absolute_result(&h, "dialog-error");
    assert_eq!(result["data"]["body"]["items"], serde_json::json!([]));
    h.step(
        port_reply_with_body(
            corr,
            Bytes::from(serde_json::json!([dialog_absolute_member_row()]).to_string()),
        ),
        2_003,
    );
    assert!(h.sink.as_slice().is_empty(), "迟到成员回包必须被忽略");
}

/// 频道分页窗口不变，成员权威置顶可以改变窗口内的最终排序。
#[test]
fn dialog_query_absolute_member_top_reorders_channel_window() {
    let mut h = Harness::new_with_identity(&[1, 2], "user-a", "company-a", "", "", "");
    let mut channels = dialog_absolute_channel_rows();
    let mut second_channel = channels[0].clone();
    channels[0]["is_top"] = serde_json::json!(1);
    channels[0]["last_post_at"] = serde_json::json!(500);
    second_channel["id"] = serde_json::json!(cid(2));
    channels.as_array_mut().unwrap().push(second_channel);
    let corr = dialog_absolute_member_corr(&mut h, "dialog-member-order", channels);
    let mut first_member = dialog_absolute_member_row();
    first_member["channel_is_top"] = serde_json::json!(0);
    first_member["last_post_at"] = serde_json::json!(500);
    let mut second_member = dialog_absolute_member_row();
    second_member["channel_id"] = serde_json::json!(cid(2));
    second_member["last_post_at"] = serde_json::json!(100);
    h.step(
        port_reply_with_body(
            corr,
            Bytes::from(serde_json::json!([first_member, second_member]).to_string()),
        ),
        2_002,
    );
    let result = dialog_absolute_result(&h, "dialog-member-order");
    let items = result["data"]["body"]["items"].as_array().unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0]["id"], cid(2));
    assert_eq!(items[0]["channelIsTop"], true);
    assert_eq!(items[1]["id"], cid(1));
    assert_eq!(items[1]["channelIsTop"], false);
}

/// 两阶段都绑定同一身份；A→B→A 后旧成员回包不能复活原查询。
#[test]
fn dialog_query_absolute_identity_aba_discards_late_member_reply() {
    let mut h = Harness::new_with_identity(&[1], "user-a", "company-a", "", "", "");
    let corr = dialog_absolute_member_corr(&mut h, "dialog-old-a", dialog_absolute_channel_rows());
    for (offset, (user, company)) in [("user-b", "company-b"), ("user-a", "company-a")]
        .into_iter()
        .enumerate()
    {
        h.step(
            query_cmd(
                helix_im::module::RUNTIME_IDENTITY_COMMAND,
                serde_json::json!({"auth_user_id": user, "company_id": company}),
            ),
            2_002 + offset as u64,
        );
    }
    h.step(
        port_reply_with_body(
            corr,
            Bytes::from(serde_json::json!([dialog_absolute_member_row()]).to_string()),
        ),
        2_004,
    );
    assert!(h.sink.as_slice().is_empty(), "旧身份成员回包必须彻底失效");
}
