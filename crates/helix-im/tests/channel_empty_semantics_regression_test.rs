//! 回归锚：群聊按需赋值。仅公开 ImModule 入站、Persist 和 Emit seam；不执行真实 I/O。
//! JSON 输入由 Go TestChannelEmptySemanticsExportCurrentRosterWithHistory 生成，非手写 canonical。

mod common;
use common::*;
use helix_core::effect::{Row, SqlValue, StorageOp, UpsertSpec};
use helix_core::tick::InboundBytes;
use helix_core::{Effect, Tick};
use serde_json::{json, Value};

/// 通过公开入站发送 WS 数据，保留真实 handler 的持久化屏障。
fn inbound(h: &mut Harness, action: &str, data: Value) {
    h.step(
        Tick::Inbound(InboundBytes(
            serde_json::to_vec(&json!({"action": action, "data": data}))
                .unwrap()
                .into(),
        )),
        1000,
    );
}

/// 只观察生产代码发出的写入计划，不在测试中复制成员合并算法。
fn upsert<'a>(h: &'a Harness, table: &str) -> &'a UpsertSpec {
    h.sink
        .as_slice()
        .iter()
        .find_map(|effect| {
            let ops = match effect {
                Effect::PersistAtomic { ops, .. } | Effect::Persist { ops, .. } => ops,
                _ => return None,
            };
            ops.iter().find_map(|op| match op {
                StorageOp::BatchUpsert(spec) if spec.table == table => Some(spec),
                _ => None,
            })
        })
        .expect("handler must produce the requested table upsert")
}

/// 获取写入计划中的命名字段。
fn field<'a>(row: &'a Row, name: &str) -> Option<&'a SqlValue> {
    row.iter()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value)
}

/// 原样回放 Go 生成的当前名单加历史加入记录，当前管理员身份不能被历史 MEMBER 覆盖。
#[test]
fn current_manager_survives_historical_join() {
    let wire: Value =
        serde_json::from_str(include_str!("channel_empty_semantics_go_input.json")).unwrap();
    let mut h = Harness::new_with_identity(&[1], "owner", "team-a", "", "", "");
    inbound(&mut h, "channel_member_update", wire["data"].clone());
    let members = upsert(&h, "channel_member");
    let row = members
        .rows
        .iter()
        .find(|row| text_field(row, "user_id") == Some("manager"))
        .unwrap();
    assert_eq!(
        text_field(row, "role"),
        Some("ADMIN"),
        "current Go roster wins over historical join role"
    );
}

/// 同一 Go 投影的精简 owner 子容器不能清空 members 中已有的权威昵称。
#[test]
fn role_summary_does_not_erase_authoritative_nickname() {
    let wire: Value =
        serde_json::from_str(include_str!("channel_empty_semantics_go_input.json")).unwrap();
    let mut h = Harness::new_with_identity(&[1], "owner", "team-a", "", "", "");
    inbound(&mut h, "channel_member_update", wire["data"].clone());
    let row = upsert(&h, "channel_member")
        .rows
        .iter()
        .find(|row| text_field(row, "user_id") == Some("owner"))
        .unwrap();
    assert_eq!(text_field(row, "nick_name"), Some("群主昵称"));
}

/// 已有频道的成员更新不应把未提供的频道设置编译为覆盖写。
#[test]
fn sparse_member_update_does_not_overwrite_unrelated_channel_fields() {
    let mut h = Harness::new_with_identity(&[1], "owner", "team-a", "", "", "");
    inbound(
        &mut h,
        "channel_member_update",
        json!({"id": cid(1), "memberChange": {"join": [{"id":"new", "teamId":"t", "role":"MEMBER"}]}}),
    );
    let patch = h
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| {
            let Effect::PersistAtomic { ops, .. } = effect else {
                return None;
            };
            ops.iter().find_map(|op| match op {
                StorageOp::BatchUpdate(spec) if spec.table == "channel" => Some(&spec.patch),
                _ => None,
            })
        })
        .expect("sparse channel must update existing rows only");
    for column in ["display_name", "orient", "picture", "props"] {
        assert!(
            field(patch, column).is_none(),
            "absent {column} must not overwrite persisted data"
        );
    }
}

/// DB PATCH 忽略 null 后，其成功事件也不得把同一 null 当新事实发出。
#[test]
fn patch_emit_does_not_clear_fields_skipped_by_persistence() {
    let mut h = Harness::new_with_identity(&[1], "owner", "team-a", "", "", "");
    inbound(
        &mut h,
        "update_channel",
        json!({"channel":{"id":cid(1),"displayName":"新群名","purpose":null,"orient":null,"props":null}}),
    );
    let corr = h
        .sink
        .as_slice()
        .iter()
        .find_map(|e| match e {
            Effect::PersistAtomic { corr, .. } => Some(corr.raw()),
            _ => None,
        })
        .unwrap();
    assert!(!h
        .sink
        .as_slice()
        .iter()
        .any(|e| matches!(e, Effect::Emit { .. })));
    h.step(port_reply_ok(corr), 1001);
    assert!(!h
        .sink
        .as_slice()
        .iter()
        .any(|e| matches!(e, Effect::Emit { .. })));
    let read = h
        .sink
        .as_slice()
        .iter()
        .find_map(|e| match e {
            Effect::Persist { corr, .. } => Some(corr.raw()),
            _ => None,
        })
        .unwrap();
    h.step(
        port_reply_with_body(
            read,
            serde_json::to_vec(
                &json!([{"id":cid(1),"snapshot_updated_at":0,"display_name":"新群名"}]),
            )
            .unwrap()
            .into(),
        ),
        1002,
    );
    let event: Value = h
        .sink
        .as_slice()
        .iter()
        .find_map(|e| match e {
            Effect::Emit { event } => {
                let value: Value = serde_json::from_slice(&event.0).unwrap();
                (value["event"] == "im:channel:update").then_some(value)
            }
            _ => None,
        })
        .unwrap();
    for key in ["purpose", "orient", "props"] {
        assert!(
            event["data"].get(key).is_none(),
            "unwritten {key} leaked into event: {event}"
        );
    }
}

/// 昵称未提供必须拒绝，不能产生清空写入。
#[test]
fn nickname_missing_does_not_persist_clear() {
    assert_invalid_nickname_does_not_write(false);
}

/// 显式 null 也不能偷换为清空字符串。
#[test]
fn nickname_null_does_not_persist_clear() {
    assert_invalid_nickname_does_not_write(true);
}

/// 通过公开入站比较两种无效昵称表示。
fn assert_invalid_nickname_does_not_write(with_null: bool) {
    let mut h = Harness::new_with_identity(&[1], "owner", "team-a", "", "", "");
    let mut data = json!({"channelId":cid(1),"userId":"owner"});
    if with_null {
        data["nickName"] = Value::Null;
    }
    inbound(&mut h, "update_channel_member_nickName", data);
    assert!(
        !h.sink
            .as_slice()
            .iter()
            .any(|e| matches!(e, Effect::PersistAtomic { .. } | Effect::Persist { .. })),
        "missing/null nickname must not become a clear command"
    );
}

/// 合法清空昵称保持有效，并且只有 matching PersistOk 后才有终态。
#[test]
fn explicit_empty_nickname_commits_and_emits() {
    let mut h = Harness::new_with_identity(&[1], "owner", "team-a", "", "", "");
    inbound(
        &mut h,
        "update_channel_member_nickName",
        json!({"channelId":cid(1),"userId":"owner","nickName":""}),
    );
    let corr = h
        .sink
        .as_slice()
        .iter()
        .find_map(|e| match e {
            Effect::PersistAtomic { corr, .. } => Some(corr.raw()),
            _ => None,
        })
        .unwrap();
    assert!(!h
        .sink
        .as_slice()
        .iter()
        .any(|e| matches!(e, Effect::Emit { .. })));
    h.step(port_reply_ok(corr), 1001);
    let events: Vec<Value> = h
        .sink
        .as_slice()
        .iter()
        .filter_map(|e| match e {
            Effect::Emit { event } => Some(serde_json::from_slice(&event.0).unwrap()),
            _ => None,
        })
        .collect();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["data"]["nickName"], "");
}

/// 将 SQL 文本值作为可比较的观察值，不改变写入计划。
fn text_field<'a>(row: &'a Row, name: &str) -> Option<&'a str> {
    match field(row, name) {
        Some(SqlValue::Text(value)) => Some(value.as_str()),
        _ => None,
    }
}

/// 取消最后一个管理员是合法权威变化，不能被“空数组一律保留”挡住。
#[test]
fn authoritative_demotion_is_not_blocked_by_empty_admins() {
    let mut h = Harness::new_with_identity(&[1], "owner", "team-a", "", "", "");
    inbound(
        &mut h,
        "channel_member_update",
        json!({"id":cid(1), "members":[{"id":"manager","teamId":"team-a","role":"MEMBER","nickName":""}],"adminUsers":[]}),
    );
    let row = &upsert(&h, "channel_member").rows[0];
    assert_eq!(text_field(row, "role"), Some("MEMBER"));
    assert_eq!(text_field(row, "nick_name"), Some(""));
}

/// 显式空字符串/空对象按现有整体替换合同有效；不能一律做 truthy 过滤或深合并。
#[test]
fn explicit_clear_patch_is_emitted_after_commit() {
    let mut h = Harness::new_with_identity(&[1], "owner", "team-a", "", "", "");
    inbound(
        &mut h,
        "update_channel",
        json!({"channel":{"id":cid(1),"purpose":"","orient":"","props":{}}}),
    );
    let corr = h
        .sink
        .as_slice()
        .iter()
        .find_map(|e| match e {
            Effect::PersistAtomic { corr, .. } => Some(corr.raw()),
            _ => None,
        })
        .unwrap();
    h.step(port_reply_ok(corr), 1001);
    assert!(!h
        .sink
        .as_slice()
        .iter()
        .any(|e| matches!(e, Effect::Emit { .. })));
    let read = h
        .sink
        .as_slice()
        .iter()
        .find_map(|e| match e {
            Effect::Persist { corr, .. } => Some(corr.raw()),
            _ => None,
        })
        .unwrap();
    h.step(port_reply_with_body(read,serde_json::to_vec(&json!([{"id":cid(1),"snapshot_updated_at":0,"purpose":"","orient":"","props":"{}"}])).unwrap().into()),1002);
    let emitted: Value = h
        .sink
        .as_slice()
        .iter()
        .find_map(|e| match e {
            Effect::Emit { event } => serde_json::from_slice::<Value>(&event.0)
                .ok()
                .filter(|v| v["event"] == "im:channel:update"),
            _ => None,
        })
        .unwrap();
    assert_eq!(emitted["data"]["purpose"], "");
    assert_eq!(emitted["data"]["orient"], "");
    assert_eq!(emitted["data"]["props"], json!({}));
}

/// 持久化失败与重复回执不得释放昵称成功终态。
#[test]
fn failed_nickname_write_does_not_emit_or_revive_on_late_success() {
    let mut h = Harness::new_with_identity(&[1], "owner", "team-a", "", "", "");
    inbound(
        &mut h,
        "update_channel_member_nickName",
        json!({"channelId":cid(1),"userId":"owner","nickName":""}),
    );
    let corr = h
        .sink
        .as_slice()
        .iter()
        .find_map(|e| match e {
            Effect::PersistAtomic { corr, .. } => Some(corr.raw()),
            _ => None,
        })
        .unwrap();
    h.step(port_reply_err(corr), 1001);
    assert!(!h
        .sink
        .as_slice()
        .iter()
        .any(|e| matches!(e, Effect::Emit { .. })));
    h.step(port_reply_ok(corr), 1002);
    assert!(!h
        .sink
        .as_slice()
        .iter()
        .any(|e| matches!(e, Effect::Emit { .. })));
}
