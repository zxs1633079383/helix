#[path = "../../support/phase1_reference.rs"]
mod phase1_reference;

use bytes::Bytes;
use helix_core::effect::Effect;
use helix_core::tick::{AppCommand, PortOutcome, ReplyBytes};
use helix_core::{Correlation, EffectSink, Module, Tick};
use helix_im::{ImConfig, ImModule};
use serde_json::{json, Value};

const GATE_ID: &str = "MV3-G09d";
const VIEWER: &str = "viewer-g09d";
const COMPANY: &str = "company-g09d";
const CHANNEL: &str = "chfixx0000000000000000003d";

// ADR-007 no-sync fixture; explicit req_id selects the Phase 3 canonical result path.
const INCREMENT_BODY_B64: &str = "eyJkYXRhIjp7ImlkIjoiY2hmaXh4MDAwMDAwMDAwMDAwMDAwMDAwM2QiLCJ0ZWFtSWQiOiJjb21wYW55LWcwOWQiLCJkaXNwbGF5TmFtZSI6IkcwOWQiLCJsYXN0RXZlbnRTZXEiOjAsIm5lZWRTeW5jIjpmYWxzZSwibWVtYmVyQ291bnQiOjQsIm1lbWJlcnMiOlt7ImlkIjoiYm9zcy1nMDlkIiwidGVhbUlkIjoiY29tcGFueS1nMDlkIiwicm9sZSI6IkJPU1MifSx7ImlkIjoidmlld2VyLWcwOWQiLCJ0ZWFtSWQiOiJjb21wYW55LWcwOWQiLCJyb2xlIjoiTUVNQkVSIn0seyJpZCI6Im93bmVyLWcwOWQiLCJ0ZWFtSWQiOiJjb21wYW55LWcwOWQiLCJyb2xlIjoiQ1JFQVRPUiJ9LHsiaWQiOiJhZG1pbi1nMDlkIiwidGVhbUlkIjoiY29tcGFueS1nMDlkIiwicm9sZSI6Ik1BTkFHRVIifV0sImFkbWluVXNlcnMiOlt7ImlkIjoiYWRtaW4tZzA5ZCIsInRlYW1JZCI6ImNvbXBhbnktZzA5ZCJ9XSwiYm9zcyI6W3siaWQiOiJib3NzLWcwOWQiLCJ0ZWFtSWQiOiJjb21wYW55LWcwOWQifV0sIm93bmVyIjp7ImlkIjoib3duZXItZzA5ZCIsInRlYW1JZCI6ImNvbXBhbnktZzA5ZCJ9LCJ0eXBlIjoiUCJ9fQ==";
const NULL_BODY_B64: &str = "eyJkYXRhIjpudWxsfQ==";

/// 通过真实 ImModule::handle 回放 G09d 的 HTTP、durable read-back 和终态 Effect。
struct Replay {
    module: ImModule,
    sink: EffectSink,
}

impl Replay {
    /// 创建带固定 RuntimeAuth 与租户的单频道回放器。
    fn new() -> Self {
        Self {
            module: ImModule::new(ImConfig {
                auth_user_id: VIEWER.to_string(),
                company_id: COMPANY.to_string(),
                api_base_url: "http://g09d.invalid/api/cses".to_string(),
                default_api_base_url: "http://g09d.invalid/api/cses".to_string(),
                ..ImConfig::default()
            }),
            sink: EffectSink::new(),
        }
    }

    /// 执行一帧并隔离本帧 Effects，避免迟到回报与上一帧混淆。
    fn step(&mut self, tick: Tick) -> Result<(), String> {
        self.sink.clear();
        self.module
            .handle(&tick, 1_700_001_800_000, &mut self.sink)
            .map_err(|error| error.to_string())
    }

    /// 发起带显式 req_id 的 G09d 命令，并校验 Go mutation body 不含 correlation。
    fn start(&mut self, req_id: &str) -> Result<Correlation, String> {
        self.step(Tick::Command(AppCommand::new(
            "im_channel_load_increment_by_channel_id",
            serde_json::to_vec(&json!({ "channel_id": CHANNEL, "req_id": req_id }))
                .map_err(|error| error.to_string())?,
        )))?;
        let Effect::Http { corr, req, .. } = self
            .sink
            .as_slice()
            .iter()
            .find(|effect| matches!(effect, Effect::Http { .. }))
            .ok_or_else(|| "G09d must emit one HTTP effect".to_string())?
        else {
            unreachable!();
        };
        assert!(req.url.ends_with("/channel/load/incrementByChannelId"));
        let body: Value = serde_json::from_slice(req.body.as_ref().expect("HTTP body"))
            .map_err(|error| error.to_string())?;
        assert_eq!(body, json!({ "channelId": CHANNEL }));
        Ok(*corr)
    }

    /// 回灌 ADR-007 HTTP 成功回包。
    fn reply_http(&mut self, corr: Correlation, body_b64: &str) -> Result<(), String> {
        self.step(Tick::PortReply {
            corr,
            outcome: PortOutcome::Ok(ReplyBytes(Bytes::from(
                serde_json::to_vec(&json!({
                    "status": 200,
                    "headers": [],
                    "body": body_b64,
                }))
                .map_err(|error| error.to_string())?,
            ))),
        })
    }

    /// 找出 channel/member 原子写的 correlation。
    fn persist_corr(&self) -> Option<Correlation> {
        self.sink.as_slice().iter().find_map(|effect| match effect {
            Effect::PersistAtomic { corr, .. } | Effect::Persist { corr, .. } => Some(*corr),
            _ => None,
        })
    }

    /// 回灌一次持久化成功回执。
    fn reply_persist_ok(&mut self, corr: Correlation) -> Result<(), String> {
        self.step(Tick::PortReply {
            corr,
            outcome: PortOutcome::Ok(ReplyBytes(Bytes::new())),
        })
    }

    /// 顺序回放 channel、完整 roster、message、cursor 四段 durable read-back。
    fn complete_readback(&mut self) -> Result<(), String> {
        for _ in 0..4 {
            let (corr, table) = self
                .sink
                .as_slice()
                .iter()
                .find_map(|effect| match effect {
                    Effect::Persist { corr, ops } => ops.first().map(|op| {
                        let table = match op {
                            helix_core::effect::StorageOp::Get(spec) => spec.table,
                            helix_core::effect::StorageOp::ScopedGet(spec) => spec.table,
                            helix_core::effect::StorageOp::Scan(spec) => spec.table,
                            _ => "",
                        };
                        (*corr, table)
                    }),
                    _ => None,
                })
                .ok_or_else(|| "expected G09d read-back Persist".to_string())?;
            let rows = match table {
                "channel" => json!([{
                    "id": CHANNEL,
                    "display_name": "G09d",
                    "type": "P",
                    "team_id": COMPANY,
                    "user_id": VIEWER,
                    "members": "[]",
                    "member_count": 4,
                    "unread_count": 3,
                }]),
                "channel_member" => json!([{
                    "channel_id": CHANNEL,
                    "user_id": VIEWER,
                    "team_id": COMPANY,
                    "role": "MEMBER",
                    "unread_count": 3,
                }, {
                    "channel_id": CHANNEL,
                    "user_id": "admin-g09d",
                    "team_id": COMPANY,
                    "role": "ADMIN",
                }, {
                    "channel_id": CHANNEL,
                    "user_id": "boss-g09d",
                    "team_id": COMPANY,
                    "role": "BOSS",
                }, {
                    "channel_id": CHANNEL,
                    "user_id": "owner-g09d",
                    "team_id": COMPANY,
                    "role": "OWNER",
                }]),
                "message" => json!([]),
                "channel_event_cursor" => json!([{
                    "channel_id": CHANNEL,
                    "last_event_seq": 0,
                }]),
                other => return Err(format!("unexpected G09d read-back table {other}")),
            };
            self.step(Tick::PortReply {
                corr,
                outcome: PortOutcome::Ok(ReplyBytes(Bytes::from(
                    serde_json::to_vec(&rows).map_err(|error| error.to_string())?,
                ))),
            })?;
        }
        Ok(())
    }

    /// 提取本帧所有 Emit，供 sender-only、顺序和零副作用断言使用。
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

    /// 证明 hydration 的投影写集合不触碰 read/unread 或频道事件事实。
    fn assert_projection_only_write_set(&self) {
        let effects = format!("{:?}", self.sink.as_slice());
        assert!(!effects.contains("GuardedBump"));
        assert!(!effects.contains("read_bits"));
        assert!(!effects.contains("channel_event"));
        assert!(!effects.contains("post_read"));
    }
}

/// 锁定 G09d 的完整只读参考包，供后续增量读取测试复用。
#[test]
fn phase3_gate_reference_is_complete() {
    phase1_reference::assert_phase3_reference_gate(GATE_ID);
}

/// 命中必须在 durable read-back 后只发一次 sender-only increment 与同 req_id Result。
#[test]
fn hydration_hit_emits_one_increment_before_one_result() -> Result<(), String> {
    let mut replay = Replay::new();
    let req_id = "g09d-explicit-1";
    let http_corr = replay.start(req_id)?;
    replay.reply_http(http_corr, INCREMENT_BODY_B64)?;
    assert!(replay.events().is_empty());
    replay.assert_projection_only_write_set();
    let persist_corr = replay
        .persist_corr()
        .ok_or_else(|| "G09d must persist before terminal output".to_string())?;
    assert!(!format!("{:?}", replay.sink.as_slice()).contains("GuardedBump"));
    replay.reply_persist_ok(persist_corr)?;
    assert!(replay.events().is_empty());
    replay.complete_readback()?;

    let events = replay.events();
    assert_eq!(events.len(), 2, "one canonical event plus one Result");
    assert_eq!(events[0]["event"], "im:channel:increment");
    assert!(events[0]["data"].is_object());
    assert_eq!(events[0]["data"]["memberCount"], 4);
    assert_eq!(
        events[0]["data"]["members"].as_array().map(Vec::len),
        Some(1)
    );
    // canonical projection 按四角色分桶；四个原始成员仍逐一核对，禁止漏人。
    assert_eq!(events[0]["data"]["members"][0]["userId"], VIEWER);
    assert_eq!(events[0]["data"]["boss"][0]["userId"], "boss-g09d");
    assert_eq!(events[0]["data"]["adminUsers"][0]["userId"], "admin-g09d");
    assert_eq!(events[0]["data"]["owner"]["userId"], "owner-g09d");
    assert_eq!(
        events[0]["data"]["adminUsers"].as_array().map(Vec::len),
        Some(1)
    );
    assert_eq!(events[0]["data"]["boss"].as_array().map(Vec::len), Some(1));
    assert!(events[0]["data"].get("req_id").is_none());
    assert_eq!(events[1]["event"], "im:read:result");
    assert_eq!(events[1]["data"]["req_id"], req_id);
    assert_eq!(events[1]["data"]["body"]["channelId"], CHANNEL);
    assert_eq!(
        events[1]["data"]["body"]["channel"]["members"],
        events[0]["data"]["members"]
    );
    assert!(replay
        .sink
        .as_slice()
        .iter()
        .all(|effect| !matches!(effect, Effect::Http { .. } | Effect::Send { .. })));
    Ok(())
}

/// 未命中必须返回单个 null Result，不能发布 channelIncrement 或写入存储。
#[test]
fn hydration_miss_emits_null_result_without_event_or_storage() -> Result<(), String> {
    let mut replay = Replay::new();
    let req_id = "g09d-explicit-miss";
    let http_corr = replay.start(req_id)?;
    replay.reply_http(http_corr, NULL_BODY_B64)?;
    let events = replay.events();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["event"], "im:read:result");
    assert_eq!(events[0]["data"]["req_id"], req_id);
    assert!(events[0]["data"]["body"].is_null());
    assert!(replay.sink.as_slice().iter().all(|effect| !matches!(
        effect,
        Effect::Persist { .. } | Effect::PersistAtomic { .. }
    )));
    Ok(())
}
