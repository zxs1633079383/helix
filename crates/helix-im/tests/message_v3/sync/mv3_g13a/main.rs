//! MV3-G13a：当前 RuntimeAuth 的本地会话列表单结果与租户隔离。

use bytes::Bytes;
use helix_core::effect::{Effect, ScanOrder, SqlValue, StorageOp};
use helix_core::tick::{AppCommand, PortError, PortOutcome, ReplyBytes};
use helix_core::{Correlation, EffectSink, Module, Tick};
use helix_im::{ImConfig, ImModule};
use serde_json::{json, Value};

const VIEWER: &str = "viewer-g13a";
const COMPANY: &str = "company-g13a";
const CHANNEL_VISIBLE: &str = "chfixx0000000000000000003a";
const CHANNEL_OTHER_TENANT: &str = "chfixx0000000000000000003b";
const CHANNEL_OTHER_VIEWER: &str = "chfixx0000000000000000003c";

/// 驱动真实 `ImModule::handle`，不旁路 command 或 PortReply owner。
struct Replay {
    module: ImModule,
    sink: EffectSink,
}

impl Replay {
    /// 创建固定 RuntimeAuth 的本地查询回放器。
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

    /// 执行单步并覆盖当前 Effect batch。
    fn step(&mut self, tick: Tick) -> Result<(), String> {
        self.sink.clear();
        self.module
            .handle(&tick, 1_700_001_800_000, &mut self.sink)
            .map_err(|error| error.to_string())
    }

    /// 发起会话列表查询并返回唯一本地 Scan correlation。
    fn query(&mut self, payload: Value) -> Result<Correlation, String> {
        self.step(Tick::Command(AppCommand::new(
            "im_query_dialog_list",
            serde_json::to_vec(&payload).map_err(|error| error.to_string())?,
        )))?;
        self.sink
            .as_slice()
            .iter()
            .find_map(|effect| match effect {
                Effect::Persist { corr, .. } => Some(*corr),
                _ => None,
            })
            .ok_or_else(|| "query must emit one local Scan".to_string())
    }

    /// 回灌本地 Scan 成功结果。
    fn reply_ok(&mut self, corr: Correlation, rows: Value) -> Result<(), String> {
        self.step(Tick::PortReply {
            corr,
            outcome: PortOutcome::Ok(ReplyBytes(Bytes::from(
                serde_json::to_vec(&rows).map_err(|error| error.to_string())?,
            ))),
        })
    }

    /// 回灌本地 Scan 失败结果。
    fn reply_err(&mut self, corr: Correlation) -> Result<(), String> {
        self.step(Tick::PortReply {
            corr,
            outcome: PortOutcome::Err(PortError::Storage(500)),
        })
    }

    /// 解码当前 batch 中的 Emit payload。
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
}

/// 运行时租户必须进入 channel Scan；请求体只允许 limit/req_id，不接受 caller identity。
#[test]
fn query_dialog_list_scans_runtime_company_without_remote_or_business_emit() {
    let mut replay = Replay::new();
    let corr = replay
        .query(json!({"limit": 60, "req_id": "req-g13a-scan"}))
        .expect("query should be accepted");

    assert_eq!(replay.sink.as_slice().len(), 1);
    let Effect::Persist {
        corr: persisted,
        ops,
    } = &replay.sink.as_slice()[0]
    else {
        panic!("dialog query must start with Persist Scan");
    };
    assert_eq!(*persisted, corr);
    let [StorageOp::Scan(spec)] = ops.as_slice() else {
        panic!("dialog query must issue exactly one Scan");
    };
    assert_eq!(spec.table, "channel");
    assert_eq!(spec.limit, Some(60));
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
    assert!(replay.events().is_empty(), "query acceptance must not emit");
    assert!(replay
        .sink
        .as_slice()
        .iter()
        .all(|effect| !matches!(effect, Effect::Http { .. } | Effect::Send { .. })));
}

/// caller 传入 team/account 不能改变 RuntimeAuth；越权身份在 I/O 前 fail-closed。
#[test]
fn query_dialog_list_rejects_caller_identity_fields_before_io() {
    for payload in [
        json!({"team_id": "company-other", "limit": 60}),
        json!({"account_id": "account-other", "limit": 60}),
        json!({"user_id": "viewer-other", "limit": 60}),
    ] {
        let mut replay = Replay::new();
        assert!(replay.query(payload).is_err());
        assert!(replay.sink.as_slice().is_empty());
    }
}

/// 缺 RuntimeAuth 时 fail-closed，不能退化到无 scope 全表 Scan。
#[test]
fn query_dialog_list_rejects_missing_runtime_identity_before_io() {
    let mut module = ImModule::new(ImConfig::default());
    let mut sink = EffectSink::new();
    let result = module.handle(
        &Tick::Command(AppCommand::new(
            "im_query_dialog_list",
            br#"{"limit":60}"#.to_vec(),
        )),
        1_700_001_800_000,
        &mut sink,
    );
    assert!(result.is_err());
    assert!(sink.as_slice().is_empty());
}

/// PortReply 只释放一个 `im:read:result` typed Result，并剔除跨租户/跨 viewer 行。
#[test]
fn query_dialog_list_emits_one_scoped_result_without_legacy_events() -> Result<(), String> {
    let mut replay = Replay::new();
    let corr = replay.query(json!({"limit": 60, "req_id": "req-g13a-result"}))?;
    replay.reply_ok(
        corr,
        json!([
            channel_row(CHANNEL_VISIBLE, COMPANY, VIEWER),
            channel_row(CHANNEL_OTHER_TENANT, "company-other", VIEWER),
            channel_row(CHANNEL_OTHER_VIEWER, COMPANY, "viewer-other"),
        ]),
    )?;

    let events = replay.events();
    assert_eq!(events.len(), 1, "one command must have one terminal Result");
    assert_eq!(events[0]["event"], "im:read:result");
    assert_eq!(events[0]["data"]["req_id"], "req-g13a-result");
    assert_eq!(
        events[0]["data"]["body"]["items"].as_array().unwrap().len(),
        1
    );
    let item = &events[0]["data"]["body"]["items"][0];
    assert_eq!(item["id"], CHANNEL_VISIBLE);
    assert_eq!(item["teamId"], COMPANY);
    let encoded = serde_json::to_string(&events).map_err(|error| error.to_string())?;
    for legacy in [
        "im:channel:list",
        "im:read:channel",
        "im:channel:increment",
        "im:channels:loaded",
    ] {
        assert!(!encoded.contains(legacy), "legacy event leaked: {legacy}");
    }
    Ok(())
}

/// Go 关群只写 delete_at；close=false 的频道仍须返回给当前 viewer。
#[test]
fn query_dialog_list_returns_archived_channel_with_delete_at() -> Result<(), String> {
    let mut replay = Replay::new();
    let corr = replay.query(json!({"req_id": "req-g13a-archive"}))?;
    replay.reply_ok(
        corr,
        json!([
            {"id": CHANNEL_VISIBLE, "team_id": COMPANY, "user_id": VIEWER,
             "is_closed": 0, "delete_at": 1789974402636_i64, "is_remove": 0},
            {"id": CHANNEL_OTHER_TENANT, "team_id": "company-other", "user_id": VIEWER,
             "is_closed": 1, "delete_at": 1789974402636_i64},
            {"id": CHANNEL_OTHER_VIEWER, "team_id": COMPANY, "user_id": "viewer-other",
             "is_closed": 1, "delete_at": 1789974402636_i64},
            {"id": "removed", "team_id": COMPANY, "user_id": VIEWER,
             "is_closed": 1, "delete_at": 1789974402636_i64, "is_remove": 1},
            {"id": "deleted", "team_id": COMPANY, "user_id": VIEWER,
             "is_closed": 0, "delete_at": 1789974402636_i64, "is_remove": 1}
        ]),
    )?;
    let events = replay.events();
    assert_eq!(events.len(), 1);
    let items = events[0]["data"]["body"]["items"]
        .as_array()
        .ok_or("missing items")?;
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["id"], CHANNEL_VISIBLE);
    assert_eq!(items[0]["close"], false);
    assert_eq!(items[0]["deleteAt"], 1789974402636_i64);
    Ok(())
}

/// 空成功与失败都返回唯一 typed empty Result，不清理既有状态或发布业务广播。
#[test]
fn query_dialog_list_empty_and_failure_are_single_typed_results() -> Result<(), String> {
    for failure in [false, true] {
        let mut replay = Replay::new();
        let corr = replay.query(json!({"req_id": "req-g13a-empty"}))?;
        if failure {
            replay.reply_err(corr)?;
        } else {
            replay.reply_ok(corr, json!([]))?;
        }
        let events = replay.events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0]["event"], "im:read:result");
        assert_eq!(events[0]["data"]["body"]["items"], json!([]));
    }
    Ok(())
}

/// corr_map remove 提供 query terminal 幂等，重复/迟到回报不产生第二个 Result。
#[test]
fn query_dialog_list_terminal_is_idempotent() -> Result<(), String> {
    let mut replay = Replay::new();
    let corr = replay.query(json!({"req_id": "req-g13a-once"}))?;
    replay.reply_ok(corr, json!([]))?;
    assert_eq!(replay.events().len(), 1);
    replay.reply_ok(corr, json!([]))?;
    assert!(replay.events().is_empty());
    Ok(())
}

/// 构造 channel storage 行；成员 JSON 同时证明当前 viewer 的本地可见性。
fn channel_row(id: &str, team_id: &str, user_id: &str) -> Value {
    json!({
        "id": id,
        "display_name": "G13a",
        "type": "P",
        "team_id": team_id,
        "user_id": user_id,
        "members": serde_json::to_string(&json!([{"userId": user_id, "role": "MEMBER"}])).unwrap(),
        "member_count": 1,
        "unread_count": 3,
        "mention_count": 1,
        "last_post_at": 1700000000000_i64,
        "created_at": 1700000000000_i64,
        "is_top": 0,
        "root_id": ""
    })
}
