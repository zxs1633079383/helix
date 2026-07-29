// 回归锚: Spec 047/T064-T065，G-13 会话已读绝对态。

use bytes::Bytes;
use helix_core::effect::Effect;
use helix_core::tick::{AppCommand, InboundBytes, PortOutcome, ReplyBytes};
use helix_core::{Correlation, EffectSink, Module, Tick};
use helix_im::{ImConfig, ImModule};
use serde_json::{json, Value};

#[path = "../../support/artifact.rs"]
mod artifact;

const CHANNEL_ID: &str = "chfixx0000000000000000001d";
const ACTOR_ID: &str = "g13-sender";
const CREATED_AT: i64 = 1_700_001_400_000;

type TestResult<T> = Result<T, String>;

/// 保存首次未读、服务端已读和冷恢复三个绝对态。
struct PersonaTrace {
    events: Vec<Value>,
    ticks: Vec<Value>,
}

/// 通过真实 `ImModule::handle` 捕获读 command、WS 与持久化回执。
struct Replay {
    module: ImModule,
    sink: EffectSink,
}

impl Replay {
    /// 创建固定 viewer 身份的 G-13 回放器。
    fn new() -> Self {
        Self {
            module: ImModule::new(ImConfig {
                api_base_url: "http://g13.invalid".to_string(),
                default_api_base_url: "http://g13.invalid".to_string(),
                auth_user_id: ACTOR_ID.to_string(),
                company_id: "g13-company".to_string(),
                ..ImConfig::default()
            }),
            sink: EffectSink::new(),
        }
    }

    /// 执行单步并覆盖当前 Effect batch。
    fn step(&mut self, tick: Tick, now_ms: u64) -> TestResult<()> {
        self.sink.clear();
        self.module
            .handle(&tick, now_ms, &mut self.sink)
            .map_err(|error| error.to_string())
    }

    /// 发起 command 并返回指定 Effect 类型的 correlation。
    fn command_corr(&mut self, name: &'static str, payload: Value) -> TestResult<Correlation> {
        let bytes = serde_json::to_vec(&payload).map_err(|error| error.to_string())?;
        self.step(
            Tick::Command(AppCommand::new(name, bytes)),
            CREATED_AT as u64,
        )?;
        self.only_corr()
    }

    /// 注入 update_channel 已读 authority 并返回 Persist correlation。
    fn ingest_read(&mut self) -> TestResult<Correlation> {
        let bytes = serde_json::to_vec(&json!({
            "action": "update_channel",
            "event_seq": 42,
            "data": {
                "channel": {
                    "id": CHANNEL_ID,
                    "userId": ACTOR_ID,
                    "unreadCount": 0,
                    "unreadPostId": "",
                    "mentionCount": 0,
                    "mentionCountRoot": 0,
                    "mentionList": [],
                    "urgentCount": 0,
                    "urgentPostList": [],
                    "lastReadSeq": 41,
                    "lastPostAt": CREATED_AT - 10,
                    "lastRootPostAt": CREATED_AT - 10,
                    "projectionRevision": 9
                }
            }
        }))
        .map_err(|error| error.to_string())?;
        self.step(
            Tick::Inbound(InboundBytes(Bytes::from(bytes))),
            CREATED_AT as u64 + 2,
        )?;
        self.only_corr()
    }

    /// 回灌相关 port 结果。
    fn reply(&mut self, corr: Correlation, bytes: Vec<u8>, now_ms: u64) -> TestResult<()> {
        self.step(
            Tick::PortReply {
                corr,
                outcome: PortOutcome::Ok(ReplyBytes(Bytes::from(bytes))),
            },
            now_ms,
        )
    }

    /// 返回当前 batch 唯一相关 I/O correlation。
    fn only_corr(&self) -> TestResult<Correlation> {
        let correlations = self
            .sink
            .as_slice()
            .iter()
            .filter_map(|effect| match effect {
                Effect::Persist { corr, .. }
                | Effect::PersistAtomic { corr, .. }
                | Effect::Http { corr, .. } => Some(*corr),
                _ => None,
            })
            .collect::<Vec<_>>();
        if correlations.len() != 1 {
            return Err(format!(
                "expected one correlation, got {}",
                correlations.len()
            ));
        }
        correlations
            .first()
            .copied()
            .ok_or_else(|| "missing correlation".to_string())
    }

    /// 解码当前 batch 中的 MessageV3 事件。
    fn events(&self) -> TestResult<Vec<Value>> {
        self.sink
            .as_slice()
            .iter()
            .filter_map(|effect| match effect {
                Effect::Emit { event } => Some(
                    serde_json::from_slice(event.0.as_ref()).map_err(|error| error.to_string()),
                ),
                _ => None,
            })
            .collect()
    }
}

/// 回放本地初始未读、服务端已读 authority 与冷启动恢复。
fn run_success() -> TestResult<PersonaTrace> {
    let mut replay = Replay::new();
    let query_corr = replay.command_corr("im_query_dialog_list", json!({}))?;
    replay.reply(
        query_corr,
        serde_json::to_vec(&vec![dialog_row(3, 1, 1)]).map_err(|error| error.to_string())?,
        CREATED_AT as u64 + 1,
    )?;
    let initial = replay.events()?;

    let view_corr = replay.command_corr(
        "im_channels_view",
        json!({"channels":[{"id":CHANNEL_ID,"isRoot":true}]}),
    )?;
    replay.reply(view_corr, http_success()?, CREATED_AT as u64 + 2)?;
    let persist_corr = replay.ingest_read()?;
    replay.reply(persist_corr, Vec::new(), CREATED_AT as u64 + 3)?;
    let terminal = replay.events()?;

    let mut recovered = Replay::new();
    let recovery_corr = recovered.command_corr("im_query_dialog_list", json!({}))?;
    recovered.reply(
        recovery_corr,
        serde_json::to_vec(&vec![dialog_row(0, 0, 0)]).map_err(|error| error.to_string())?,
        CREATED_AT as u64 + 4,
    )?;
    let recovery = recovered.events()?;
    let mut events = initial;
    events.extend(terminal);
    events.extend(recovery);
    Ok(PersonaTrace {
        events,
        ticks: vec![
            json!({"kind":"Command","value":{"name":"im_query_dialog_list"},"now_ms":CREATED_AT}),
            json!({"kind":"PortReply","value":{"correlation":query_corr.raw(),"outcome":{"variant":"Ok"}},"now_ms":CREATED_AT + 1}),
            json!({"kind":"Command","value":{"name":"im_channels_view","payload":{"channels":[{"id":CHANNEL_ID,"isRoot":true}]}},"now_ms":CREATED_AT + 1}),
            json!({"kind":"Inbound","value":{"action":"update_channel","event_seq":42},"now_ms":CREATED_AT + 2}),
            json!({"kind":"PortReply","value":{"correlation":persist_corr.raw(),"outcome":{"variant":"Ok"}},"now_ms":CREATED_AT + 3}),
            json!({"kind":"Command","value":{"name":"im_query_dialog_list","recovery":true},"now_ms":CREATED_AT + 3}),
            json!({"kind":"PortReply","value":{"correlation":recovery_corr.raw(),"outcome":{"variant":"Ok"}},"now_ms":CREATED_AT + 4}),
        ],
    })
}

/// 构造本地 dialog scan 的真实 channel 行。
fn dialog_row(unread: i64, mention: i64, urgent: i64) -> Value {
    json!({
        "id": CHANNEL_ID,
        "display_name": "G13",
        "type": "P",
        "team_id": "g13-company",
        "unread_count": unread,
        "mention_count": mention,
        "urgent_count": urgent,
        "last_read_seq": if unread == 0 { 41 } else { 38 },
        "projection_revision": 9,
        "last_post_at": CREATED_AT - 10
    })
}

/// 构造 channels/view 的 accepted HTTP envelope。
fn http_success() -> TestResult<Vec<u8>> {
    let body = serde_json::to_vec(&json!({"status":"SUCCESS","data":null}))
        .map_err(|error| error.to_string())?;
    serde_json::to_vec(&json!({
        "status": 200,
        "headers": {},
        "body": base64_std(&body)
    }))
    .map_err(|error| error.to_string())
}

/// 使用标准表编码测试 HTTP body。
fn base64_std(bytes: &[u8]) -> String {
    const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut output = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let first = u32::from(chunk[0]);
        let second = chunk.get(1).copied().map_or(0, u32::from);
        let third = chunk.get(2).copied().map_or(0, u32::from);
        let encoded = (first << 16) | (second << 8) | third;
        output.push(TABLE[((encoded >> 18) & 63) as usize] as char);
        output.push(TABLE[((encoded >> 12) & 63) as usize] as char);
        output.push(
            chunk
                .get(1)
                .map_or('=', |_| TABLE[((encoded >> 6) & 63) as usize] as char),
        );
        output.push(
            chunk
                .get(2)
                .map_or('=', |_| TABLE[(encoded & 63) as usize] as char),
        );
    }
    output
}

/// G-13 初始未读、已读终态与恢复都使用同一绝对事件。
#[test]
fn g13_emits_initial_terminal_and_recovery_states() -> TestResult<()> {
    let trace = run_success()?;
    assert_eq!(trace.events.len(), 6);
    assert_eq!(trace.events[0]["event"], "im:channel:list");
    assert_eq!(trace.events[1]["event"], "im:read:channel");
    assert_eq!(trace.events[1]["data"]["state"], "unread");
    assert_eq!(trace.events[1]["data"]["unreadCount"], 3);
    assert_eq!(trace.events[2]["event"], "im:channel:update");
    assert_eq!(trace.events[3]["event"], "im:read:channel");
    assert_eq!(trace.events[3]["data"]["state"], "read");
    assert_eq!(trace.events[3]["data"]["unreadCount"], 0);
    assert_eq!(trace.events[3]["data"]["lastReadSeq"], 41);
    assert_eq!(trace.events[4]["event"], "im:channel:list");
    assert_eq!(trace.events[5]["event"], "im:read:channel");
    assert_eq!(trace.events[5]["data"]["state"], "read");
    assert_eq!(trace.events[5]["data"]["unreadCount"], 0);
    Ok(())
}

/// G-13 输出不再含 paired Frame 或 renderer mutation。
#[test]
fn g13_has_no_frame_or_surface_payload() -> TestResult<()> {
    let encoded =
        serde_json::to_string(&run_success()?.events).map_err(|error| error.to_string())?;
    for forbidden in ["InteractionFrame", "Surface", "mutation", "directive"] {
        assert!(!encoded.contains(forbidden), "forbidden field {forbidden}");
    }
    Ok(())
}

/// Rust producer 在生成模式下写出 G-13 sender 与空 receiver canonical artifact。
#[test]
fn artifact_test() -> TestResult<()> {
    let trace = run_success()?;
    let Some(output) = std::env::var_os("HELIX_GATE_ARTIFACT_DIR") else {
        return Ok(());
    };
    let bundle = artifact::gate_bundle(
        "G-13",
        "message_v3_g13",
        trace.ticks,
        trace.events,
        Vec::new(),
        json!([{"table":"channel_member","columns":["channel_id","user_id","unread_count","mention_count","urgent_mention_count","last_read_seq"]}]),
        json!([{"table":"channel_member","columns":["channel_id","user_id","unread_count","mention_count","urgent_mention_count","last_read_seq"]}]),
        json!([{"table":"channel_member","columns":["channel_id","user_id","unread_count","mention_count","urgent_mention_count","last_read_seq"]}]),
    );
    artifact::write_bundle(std::path::Path::new(&output), &bundle)
        .map_err(|error| error.to_string())
        .map(|_| ())
}
