//! MV3-G13d：当前账号本地可见父频道下的 topic 纯读查询。
//!
//! 测试只驱动真实 `ImModule::handle` 与 PortReply owner，验证父频道可见性、topic
//! 关联及 single-terminal 结果；不得触发 Go/HTTP、hydration 或业务事件。

#[path = "../../../common.rs"]
mod common;

use bytes::Bytes;
use common::{cid, EffectKind};
use helix_core::effect::{Effect, ScanOrder, SqlValue, StorageOp};
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
        })
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
    let [StorageOp::Scan(spec)] = ops.as_slice() else {
        return Err("subtopic query must issue exactly one Scan".to_string());
    };
    assert_eq!(spec.table, "channel");
    assert!(matches!(
        &spec.filter,
        Some(("team_id", SqlValue::Text(value))) if value == COMPANY
    ));
    assert_eq!(
        spec.order_by,
        &[
            ScanOrder::desc("is_top"),
            ScanOrder::desc("last_post_at"),
            ScanOrder::desc("created_at"),
        ]
    );
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
    assert_single_typed_result(&replay, "req-g13d-filter", &[TOPIC_VISIBLE]);
    let event = replay.events().pop().expect("one result");
    let item = &event["data"]["body"]["items"][0];
    assert_eq!(item["type"], "T");
    assert_eq!(item["rootId"], PARENT);
    assert!(item["members"].as_array().is_some_and(|members| {
        members
            .iter()
            .any(|member| member["userId"] == "topic-owner")
    }));
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
            channel_row(PARENT, "company-other", VIEWER, "P", "", &[VIEWER]),
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

/// Scan 失败也只返回一次 typed empty Result，且重复回包不能二次结算。
#[test]
fn query_subtopics_failure_is_empty_and_terminal_is_idempotent() -> Result<(), String> {
    let mut replay = Replay::new();
    let corr = replay.query(PARENT, "req-g13d-failure")?;
    replay.reply_err(corr)?;
    assert_single_typed_result(&replay, "req-g13d-failure", &[]);
    replay.reply_ok(corr, json!([]))?;
    assert!(
        replay.events().is_empty(),
        "late reply must not emit a second Result"
    );
    Ok(())
}
