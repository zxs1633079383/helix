//! MV3-G13d：当前账号本地可见父频道下的 topic 纯读查询。
//!
//! 测试只驱动真实 `ImModule::handle` 与 PortReply owner，验证父频道可见性、topic
//! 关联及 single-terminal 结果；不得触发 Go/HTTP、hydration 或业务事件。

#[path = "../../../common.rs"]
mod common;

use bytes::Bytes;
use common::{cid, EffectKind};
use helix_core::effect::{Effect, SqlValue, StorageOp};
use helix_core::tick::{AppCommand, PortError, PortOutcome, ReplyBytes};
use helix_core::{Correlation, EffectSink, Module, Tick};
use helix_im::{ImConfig, ImModule};
use serde_json::{json, Value};

const VIEWER: &str = "viewer-g13d";
const COMPANY: &str = "company-g13d";
const PARENT: &str = "chfixx00000000000000000064";
const TOPIC_VISIBLE: &str = "chfixx00000000000000000065";

/// 驱动固定 RuntimeAuth 的 Helix 查询链，并保存当前 Effect batch。
struct Replay {
    module: ImModule,
    sink: EffectSink,
}

impl Replay {
    /// 创建当前账号与租户均固定的本地查询回放器。
    fn new() -> Self {
        Self {
            module: ImModule::new(ImConfig {
                auth_user_id: VIEWER.to_string(),
                api_base_url: "http://topic-regression.invalid".to_string(),
                company_id: COMPANY.to_string(),
                ..ImConfig::default()
            }),
            sink: EffectSink::new(),
        }
    }

    /// 执行一个真实 Module tick，并覆盖上一步的 Effect batch。
    fn step(&mut self, tick: Tick) -> Result<(), String> {
        self.sink.clear();
        self.module
            .handle(&tick, 1_700_002_000_000, &mut self.sink)
            .map_err(|error| error.to_string())
    }

    /// 发起 subtopic 查询并返回唯一 local Scan correlation。
    fn query(&mut self, parent_channel_id: &str, req_id: &str) -> Result<Correlation, String> {
        self.step(Tick::Command(AppCommand::new(
            "im_query_subtopics",
            serde_json::to_vec(&json!({
                "parentChannelId": parent_channel_id,
                "req_id": req_id,
            }))
            .map_err(|error| error.to_string())?,
        )))?;
        self.sink
            .as_slice()
            .iter()
            .find_map(|effect| match effect {
                Effect::Persist { corr, .. } => Some(*corr),
                _ => None,
            })
            .ok_or_else(|| "subtopic query must issue one local Scan".to_string())
    }

    /// 回灌 local Scan rows，保持 reply bytes 与 storage driver 约定一致。
    fn reply_ok(&mut self, corr: Correlation, rows: Value) -> Result<(), String> {
        self.step(Tick::PortReply {
            corr,
            outcome: PortOutcome::Ok(ReplyBytes(Bytes::from(
                serde_json::to_vec(&rows).map_err(|error| error.to_string())?,
            ))),
        })?;
        let next = self.sink.as_slice().iter().find_map(|effect| match effect {
            Effect::Persist { corr, ops } => Some((*corr, ops.clone())),
            _ => None,
        });
        if let Some((next_corr, ops)) = next {
            let [StorageOp::ScopedScan(spec)] = ops.as_slice() else {
                return Err("parent validation must continue with scoped topic scan".to_string());
            };
            assert_eq!(spec.scope_col, "root_id");
            assert!(
                matches!(spec.scope_values.as_slice(), [SqlValue::Text(value)] if value == PARENT)
            );
            assert!(
                matches!(&spec.filter, Some(("user_id", SqlValue::Text(value))) if value == VIEWER)
            );
            let topics: Vec<Value> = rows
                .as_array()
                .into_iter()
                .flatten()
                .filter(|row| row["root_id"] == PARENT)
                .cloned()
                .collect();
            self.step(Tick::PortReply {
                corr: next_corr,
                outcome: PortOutcome::Ok(ReplyBytes(Bytes::from(
                    serde_json::to_vec(&topics).map_err(|error| error.to_string())?,
                ))),
            })?;
        }
        Ok(())
    }

    /// 回灌失败结果，验证失败分支仍只结算空 typed Result。
    fn reply_err(&mut self, corr: Correlation) -> Result<(), String> {
        self.step(Tick::PortReply {
            corr,
            outcome: PortOutcome::Err(PortError::Storage(500)),
        })
    }

    /// 解码当前 batch 中的 Emit payload，供结果与业务事件边界断言。
    fn events(&self) -> Vec<Value> {
        self.sink
            .as_slice()
            .iter()
            .filter_map(|effect| match effect {
                Effect::Emit { event } => serde_json::from_slice(event.0.as_ref()).ok(),
                _ => None,
            })
            .collect()
    }

    /// 返回当前 batch 的简化 Effect 分类，便于断言无 HTTP/Send。
    fn effect_kinds(&self) -> Vec<EffectKind> {
        self.sink
            .as_slice()
            .iter()
            .map(EffectKind::from_effect)
            .collect()
    }
}

/// 构造 channel storage row，members 使用 driver 可能返回的 JSON text 形态。
fn channel_row(
    id: &str,
    team_id: &str,
    owner_id: &str,
    channel_type: &str,
    root_id: &str,
    members: &[&str],
) -> Value {
    let root_post_id = (channel_type == "T").then(|| format!("root-post-{id}"));
    json!({
        "id": id,
        "display_name": "G13d",
        "type": channel_type,
        "team_id": team_id,
        "user_id": owner_id,
        "members": serde_json::to_string(
            &members.iter().map(|id| json!({"userId": id})).collect::<Vec<_>>(),
        ).expect("members fixture must serialize"),
        "member_count": members.len() as i64,
        "root_id": root_id,
        "root_post_id": root_post_id,
        "created_at": 1_700_002_000_000_i64,
        "last_post_at": 1_700_002_000_000_i64,
        "is_top": 0,
    })
}

/// 只接受一个 terminal `im:read:result`，并拒绝 HTTP/WS/旧业务 Emit。
fn assert_single_typed_result(replay: &Replay, req_id: &str, expected_ids: &[&str]) {
    let events = replay.events();
    assert_eq!(
        events.len(),
        1,
        "query must emit exactly one terminal Result"
    );
    assert_eq!(events[0]["event"], "im:read:result");
    assert_eq!(events[0]["data"]["req_id"], req_id);
    let items = events[0]["data"]["body"]["items"]
        .as_array()
        .expect("typed Result must contain items array");
    let actual_ids = items
        .iter()
        .filter_map(|item| item["id"].as_str())
        .collect::<Vec<_>>();
    assert_eq!(actual_ids, expected_ids);
    let encoded = serde_json::to_string(&events).expect("events serialize");
    for forbidden in [
        "im:channel:list",
        "im:channels:loaded",
        "im:channel:increment",
        "im:message",
    ] {
        assert!(
            !encoded.contains(forbidden),
            "business event leaked: {forbidden}"
        );
    }
    assert!(replay.effect_kinds().iter().all(|kind| !matches!(
        kind,
        EffectKind::Http(_) | EffectKind::HttpFire | EffectKind::Send
    )));
}

/// 查询必须在 RuntimeAuth 租户内做一次 channel Scan，不接受 caller scope。
#[test]
fn query_subtopics_scans_runtime_company_once_without_http() -> Result<(), String> {
    let mut replay = Replay::new();
    let corr = replay.query(PARENT, "req-g13d-scan")?;
    assert_eq!(replay.sink.as_slice().len(), 1);
    let Effect::Persist {
        corr: persisted,
        ops,
    } = &replay.sink.as_slice()[0]
    else {
        return Err("subtopic query must start with Persist Scan".to_string());
    };
    assert_eq!(*persisted, corr);
    let [StorageOp::Get(spec)] = ops.as_slice() else {
        return Err("subtopic query must get its parent before a parent-scoped scan".to_string());
    };
    assert_eq!(spec.table, "channel");
    assert_eq!(spec.key_col, "id");
    assert!(matches!(&spec.key_val, SqlValue::Text(value) if value == PARENT));
    assert!(replay.events().is_empty());
    assert!(replay.effect_kinds().iter().all(|kind| !matches!(
        kind,
        EffectKind::Http(_) | EffectKind::HttpFire | EffectKind::Send
    )));
    Ok(())
}

/// 只返回当前可见父频道下 type=T 且 rootId 匹配的 topic，排除其余行。
#[test]
fn query_subtopics_filters_parent_type_root_and_account_visibility() -> Result<(), String> {
    let mut replay = Replay::new();
    let corr = replay.query(PARENT, "req-g13d-filter")?;
    replay.reply_ok(
        corr,
        json!([
            channel_row(PARENT, COMPANY, VIEWER, "P", "", &[VIEWER]),
            channel_row(
                TOPIC_VISIBLE,
                COMPANY,
                "topic-owner",
                "T",
                PARENT,
                &[VIEWER]
            ),
            channel_row(&cid(102), COMPANY, "topic-owner", "C", PARENT, &[VIEWER]),
            channel_row(&cid(103), COMPANY, "topic-owner", "T", &cid(101), &[VIEWER]),
            channel_row(
                &cid(104),
                COMPANY,
                "other-owner",
                "T",
                PARENT,
                &["other-viewer"]
            ),
            channel_row(&cid(105), "company-other", VIEWER, "T", PARENT, &[VIEWER]),
            channel_row(&cid(106), COMPANY, "topic-owner", "T", &cid(999), &[VIEWER]),
        ]),
    )?;
    // 成员授权跨公司仍可读；仅其他账号且无成员资格的行不可见。
    assert_single_typed_result(&replay, "req-g13d-filter", &[TOPIC_VISIBLE, &cid(105)]);
    let event = replay.events().pop().expect("one result");
    let item = &event["data"]["body"]["items"][0];
    assert_eq!(item["type"], "T");
    assert_eq!(item["rootId"], PARENT);
    assert_eq!(item["rootPostId"], format!("root-post-{TOPIC_VISIBLE}"));
    assert!(item["members"].as_array().is_some_and(|members| {
        members
            .iter()
            .any(|member| member["userId"] == "topic-owner")
    }));
    assert_eq!(item["memberCount"], 2);
    Ok(())
}

/// 子话题投影继承父频道的增量同步水位，避免客户端把已完成同步误判为未就绪。
#[test]
fn query_subtopics_projects_parent_sync_watermark_onto_children() -> Result<(), String> {
    let mut replay = Replay::new();
    let corr = replay.query(PARENT, "req-g13d-watermark")?;
    let mut parent = channel_row(PARENT, COMPANY, VIEWER, "P", "", &[VIEWER]);
    parent["subtopics_loaded_at"] = json!(1_700_002_000_123_i64);
    let mut topic = channel_row(
        TOPIC_VISIBLE,
        COMPANY,
        "topic-owner",
        "T",
        PARENT,
        &[VIEWER],
    );
    topic["subtopics_loaded_at"] = json!(0);

    replay.reply_ok(corr, json!([parent, topic]))?;

    assert_single_typed_result(&replay, "req-g13d-watermark", &[TOPIC_VISIBLE]);
    let event = replay.events().pop().expect("one result");
    assert_eq!(
        event["data"]["body"]["items"][0]["subtopicsLoadedAt"],
        1_700_002_000_123_i64
    );
    Ok(())
}

/// 纯数字字符串身份不能被 JSON text 解码成 number，否则创建人筛选会漏掉真实话题。
#[test]
fn query_subtopics_preserves_numeric_creator_ids_as_strings() -> Result<(), String> {
    let mut replay = Replay::new();
    let corr = replay.query(PARENT, "req-g13d-creator-id")?;
    let mut topic = channel_row(TOPIC_VISIBLE, COMPANY, "444", "T", PARENT, &[VIEWER]);
    topic["create_by"] = json!("444");
    replay.reply_ok(
        corr,
        json!([
            channel_row(PARENT, COMPANY, VIEWER, "P", "", &[VIEWER]),
            topic,
        ]),
    )?;

    assert_single_typed_result(&replay, "req-g13d-creator-id", &[TOPIC_VISIBLE]);
    let event = replay.events().pop().expect("one result");
    let item = &event["data"]["body"]["items"][0];
    assert_eq!(item["userId"], "444");
    assert_eq!(item["createBy"], "444");
    Ok(())
}

/// 父频道不可见时，即使 topic 行本身可见，也必须 fail-closed 为空。
#[test]
fn query_subtopics_requires_visible_parent() -> Result<(), String> {
    let mut replay = Replay::new();
    let corr = replay.query(PARENT, "req-g13d-parent")?;
    replay.reply_ok(
        corr,
        json!([
            channel_row(PARENT, COMPANY, "other-owner", "P", "", &["other-viewer"]),
            channel_row(
                TOPIC_VISIBLE,
                COMPANY,
                "topic-owner",
                "T",
                PARENT,
                &[VIEWER]
            ),
        ]),
    )?;
    assert_single_typed_result(&replay, "req-g13d-parent", &[]);
    Ok(())
}

/// 空、孤儿、跨账号或 remote-only parent 都返回空，不触发创建/hydration/远程链。
#[test]
fn query_subtopics_empty_or_orphan_or_remote_parent_is_typed_empty() -> Result<(), String> {
    // 空 parent 在命令入口直接终止，不应扫描整张 channel 表。
    let mut empty = Replay::new();
    empty.step(Tick::Command(AppCommand::new(
        "im_query_subtopics",
        serde_json::to_vec(&json!({ "parentChannelId": "", "req_id": "req-g13d-empty" }))
            .map_err(|error| error.to_string())?,
    )))?;
    assert_single_typed_result(&empty, "req-g13d-empty", &[]);
    assert!(!empty
        .effect_kinds()
        .iter()
        .any(|kind| matches!(kind, EffectKind::Persist(_))));

    // 合法但不在本地回复集的 parent 属于 orphan/remote-only，回包仍为空。
    let mut orphan = Replay::new();
    let corr = orphan.query(&cid(101), "req-g13d-orphan")?;
    orphan.reply_ok(corr, json!([]))?;
    assert_single_typed_result(&orphan, "req-g13d-orphan", &[]);

    // 跨账号 parent 即便伪造出 topic 行，也不能穿透 RuntimeAuth 作用域。
    let mut cross_account = Replay::new();
    let corr = cross_account.query(PARENT, "req-g13d-cross")?;
    cross_account.reply_ok(
        corr,
        json!([
            channel_row(
                PARENT,
                "company-other",
                "other-viewer",
                "P",
                "",
                &["other-viewer"]
            ),
            channel_row(
                TOPIC_VISIBLE,
                "company-other",
                VIEWER,
                "T",
                PARENT,
                &[VIEWER]
            ),
        ]),
    )?;
    assert_single_typed_result(&cross_account, "req-g13d-cross", &[]);
    Ok(())
}

/// Scan 失败返回关联错误，不能伪装成空清单成功；重复回包不能二次结算。
#[test]
fn query_subtopics_failure_is_empty_and_terminal_is_idempotent() -> Result<(), String> {
    let mut replay = Replay::new();
    let corr = replay.query(PARENT, "req-g13d-failure")?;
    replay.reply_err(corr)?;
    let events = replay.events();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["data"]["req_id"], "req-g13d-failure");
    assert!(events[0]["data"]["error"].is_string());
    assert!(events[0]["data"].get("body").is_none());
    replay.reply_ok(corr, json!([]))?;
    assert!(
        replay.events().is_empty(),
        "late reply must not emit a second Result"
    );
    Ok(())
}

/// 完整空清单仍需原子落库成功；失败、重复回执不得发布 ready。
#[test]
fn topic_snapshot_ready_requires_matching_atomic_success() -> Result<(), String> {
    for (success, superseded) in [(false, false), (true, false), (true, true)] {
        let mut replay = Replay::new();
        replay.step(Tick::Command(AppCommand::new(
            "im_query_dialog_list",
            b"{}".to_vec(),
        )))?;
        let parent_corr = replay
            .sink
            .as_slice()
            .iter()
            .find_map(|effect| match effect {
                Effect::Persist { corr, .. } => Some(*corr),
                _ => None,
            })
            .ok_or("missing parent scope query")?;
        replay.step(Tick::PortReply {
            corr: parent_corr,
            outcome: PortOutcome::Ok(ReplyBytes(Bytes::from(
                json!([channel_row(PARENT, COMPANY, VIEWER, "P", "", &[VIEWER])]).to_string(),
            ))),
        })?;
        replay.step(Tick::Inbound(helix_core::tick::InboundBytes(Bytes::from(
            json!({
                "action":"increment_channel_end", "data":{
                    "scope":"subtopics", "channelId":PARENT, "batchId":"empty-batch",
                    "requestId":"request-1", "snapshotComplete":true, "channelCount":0
                }
            })
            .to_string(),
        ))))?;
        assert!(replay.events().is_empty());
        let [Effect::Persist {
            corr: guard_read,
            ops: guard_ops,
        }] = replay.sink.as_slice()
        else {
            return Err("snapshot must read durable parent version before replacement".into());
        };
        assert!(matches!(guard_ops.as_slice(), [StorageOp::Get(_)]));
        let guard_read = *guard_read;
        replay.step(Tick::PortReply {
            corr: guard_read,
            outcome: PortOutcome::Ok(ReplyBytes(Bytes::from(
                json!([{"id":PARENT,"subtopics_loaded_at":100}]).to_string(),
            ))),
        })?;
        let [Effect::PersistAtomic { corr, ops }] = replay.sink.as_slice() else {
            return Err("snapshot must commit one atomic transaction".into());
        };
        let corr = *corr;
        assert!(ops.iter().any(|op|matches!(op, StorageOp::BatchUpsert(spec) if spec.table=="channel" && spec.update_guard.as_ref().is_some_and(|guard| guard.column=="subtopics_loaded_at"))));
        let writes = format!("{ops:?}");
        assert!(
            writes.contains("subtopics_loaded_at")
                && writes.contains("thread_count")
                && writes.contains("is_remove")
        );
        if superseded {
            replay.step(Tick::Command(AppCommand::new(
                "im_request_channel_increment",
                json!({
                    "channel_id":PARENT, "sync_request_id":"newer-request", "cursors":[]
                })
                .to_string()
                .into_bytes(),
            )))?;
        }
        replay.step(Tick::PortReply {
            corr,
            outcome: if success {
                PortOutcome::Ok(ReplyBytes(Bytes::new()))
            } else {
                PortOutcome::Err(PortError::Storage(500))
            },
        })?;
        let events = replay.events();
        let ready: Vec<_> = events
            .iter()
            .filter(|e| e["event"] == "im:subtopics-sync-ready")
            .collect();
        assert_eq!(
            ready.len(),
            usize::from(success && !superseded),
            "effects={:?}",
            replay.sink.as_slice()
        );
        if success && !superseded {
            assert_eq!(ready[0]["data"]["requestId"], "request-1");
            assert_eq!(ready[0]["data"]["snapshotComplete"], true);
            assert!(events
                .iter()
                .any(|e| e["event"] == "im:channel:update" && e["data"]["threadCount"] == 0));
        }
        replay.step(Tick::PortReply {
            corr,
            outcome: PortOutcome::Ok(ReplyBytes(Bytes::new())),
        })?;
        assert!(replay.events().is_empty());
    }
    Ok(())
}

/// 新请求已经注册后，旧请求的空清单不能清掉当前缓存。
#[test]
fn stale_topic_request_end_must_not_persist() -> Result<(), String> {
    let mut replay = Replay::new();
    replay.step(Tick::Command(AppCommand::new(
        "im_request_channel_increment",
        json!({
            "channel_id":PARENT, "sync_request_id":"new-request", "cursors":[]
        })
        .to_string()
        .into_bytes(),
    )))?;
    replay.step(Tick::Inbound(helix_core::tick::InboundBytes(Bytes::from(
        json!({
            "action":"increment_channel_end", "data":{
                "scope":"subtopics", "channelId":PARENT, "batchId":"old-batch",
                "requestId":"old-request", "snapshotComplete":true, "channelCount":0
            }
        })
        .to_string(),
    ))))?;
    assert!(
        replay.sink.as_slice().is_empty(),
        "old request must not persist: {:?}",
        replay.sink.as_slice()
    );
    Ok(())
}
