// 回归锚: Spec 047/T054-T055，G-10c 满窗口 append 与头部淘汰。

use bytes::Bytes;
use helix_core::effect::{Effect, StorageOp};
use helix_core::tick::{AppCommand, PortOutcome, ReplyBytes};
use helix_core::{Correlation, EffectSink, Module, Tick};
use helix_im::module::ImConfig;
use helix_im::query::LocalStoreMode;
use helix_im::state::ChannelId;
use helix_im::ImModule;
use serde_json::{json, Value};

#[path = "../../support/artifact.rs"]
mod artifact;

const CHANNEL_ID: &str = "chfixx0000000000000000001c";
const WINDOW_TOKEN: &str = "g10c-latest-window";
const ACTOR_ID: &str = "g10c-actor";
const CREATED_AT: i64 = 1_700_001_300_000;

type TestResult<T> = Result<T, String>;

/// 记录 sender 本地窗口更新和用于复现的真实 Tick。
struct PersonaTrace {
    events: Vec<Value>,
    ticks: Vec<Value>,
}

/// 通过公开 Module trait 驱动首屏、发送持久化和 attached refresh。
pub(crate) struct Replay {
    module: ImModule,
    sink: EffectSink,
    now_ms: u64,
}

impl Replay {
    /// 创建带 Session store 和确定性 viewer 的回放器。
    pub(crate) fn new() -> TestResult<Self> {
        let mut module = ImModule::new_with_local_store(
            ImConfig {
                api_base_url: "http://g10c.invalid".to_string(),
                default_api_base_url: "http://g10c.invalid".to_string(),
                auth_user_id: ACTOR_ID.to_string(),
                company_id: "g10c-company".to_string(),
                ..ImConfig::default()
            },
            LocalStoreMode::Session,
        );
        let channel_id = ChannelId::from_str(CHANNEL_ID)
            .ok_or_else(|| "invalid G-10c channel fixture".to_string())?;
        module.register_channel(channel_id, 0);
        Ok(Self {
            module,
            sink: EffectSink::new(),
            now_ms: CREATED_AT as u64,
        })
    }

    /// 建立恰好二十条消息的 attached latest window。
    pub(crate) fn attach_full_window(&mut self) -> TestResult<()> {
        self.command(
            "im_query_messages_by_channel",
            json!({
                "channel_id": CHANNEL_ID,
                "limit": 20,
                "windowToken": WINDOW_TOKEN,
            }),
        )?;
        let scan = self.only_corr("scan", None)?;
        self.reply(
            scan,
            PortOutcome::Ok(ReplyBytes(Bytes::from_static(b"[]"))),
            1,
        )?;
        let http = self.only_corr("http", Some("/posts/getLatestPost"))?;
        self.reply(
            http,
            PortOutcome::Ok(ReplyBytes(http_envelope(remote_page_body()?)?)),
            2,
        )?;
        let cache = self.only_corr("write", None)?;
        self.reply(cache, PortOutcome::Ok(ReplyBytes::default()), 3)?;
        let events = self.events()?;
        if events.len() != 1 || events[0]["event"] != "im:timeline:ready" {
            return Err("G-10c prerequisite must attach one ready window".to_string());
        }
        Ok(())
    }

    /// 发起普通文本发送并返回 optimistic message 持久化 correlation。
    pub(crate) fn start_send(&mut self) -> TestResult<Correlation> {
        self.command(
            "im_send_message",
            json!({
                "channel_id": CHANNEL_ID,
                "text": "G-10c appended message",
                "req_id": "g10c-send-request",
            }),
        )?;
        self.only_corr("persist", None)
    }

    /// 提交 JSON command 并执行真实 handle。
    pub(crate) fn command(&mut self, name: &'static str, payload: Value) -> TestResult<()> {
        let bytes = serde_json::to_vec(&payload).map_err(|error| error.to_string())?;
        self.step(Tick::Command(AppCommand::new(name, bytes)))
    }

    /// 回灌端口结果并推进确定性时钟。
    pub(crate) fn reply(
        &mut self,
        corr: Correlation,
        outcome: PortOutcome,
        offset_ms: u64,
    ) -> TestResult<()> {
        self.now_ms = CREATED_AT as u64 + offset_ms;
        self.step(Tick::PortReply { corr, outcome })
    }

    /// 通过真实 `ImModule::handle` 执行一步。
    fn step(&mut self, tick: Tick) -> TestResult<()> {
        self.sink.clear();
        self.module
            .handle(&tick, self.now_ms, &mut self.sink)
            .map_err(|error| error.to_string())
    }

    /// 按 Effect 类型提取唯一 correlation。
    pub(crate) fn only_corr(&self, kind: &str, suffix: Option<&str>) -> TestResult<Correlation> {
        let mut correlations = self.sink.as_slice().iter().filter_map(|effect| {
            let matched = match (kind, effect) {
                ("http", Effect::Http { req, .. }) => {
                    suffix.is_none_or(|suffix| req.url.ends_with(suffix))
                }
                ("scan", Effect::Persist { ops, .. }) => {
                    ops.iter().any(|op| matches!(op, StorageOp::Scan(_)))
                }
                ("write", Effect::Persist { ops, .. }) => ops.iter().any(
                    |op| matches!(op, StorageOp::BatchUpsert(spec) if spec.table == "message"),
                ),
                ("readback", Effect::Persist { ops, .. }) => ops
                    .iter()
                    .any(|op| matches!(op, StorageOp::Get(spec) if spec.table == "message")),
                ("persist", Effect::Persist { .. }) => true,
                _ => false,
            };
            matched.then(|| match effect {
                Effect::Http { corr, .. } | Effect::Persist { corr, .. } => *corr,
                _ => Correlation::from_raw(0),
            })
        });
        let first = correlations
            .next()
            .ok_or_else(|| format!("missing {kind} correlation"))?;
        if correlations.next().is_some() {
            return Err(format!("multiple {kind} correlations"));
        }
        Ok(first)
    }

    /// 从当前 batch 解出全部业务事件。
    pub(crate) fn events(&self) -> TestResult<Vec<Value>> {
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

/// 回放满窗口追加并返回持久读回后的唯一 timeline update。
fn run_success() -> TestResult<PersonaTrace> {
    let mut replay = Replay::new()?;
    replay.attach_full_window()?;
    let send_persist = replay.start_send()?;
    replay.reply(send_persist, PortOutcome::Ok(ReplyBytes::default()), 10)?;
    let refresh = replay.only_corr("scan", None)?;
    replay.reply(
        refresh,
        PortOutcome::Ok(ReplyBytes(local_rows_after_append()?)),
        11,
    )?;
    Ok(PersonaTrace {
        events: replay.events()?,
        ticks: vec![
            json!({
                "kind": "Command",
                "value": {"name": "im_send_message", "payload": {
                    "channel_id": CHANNEL_ID,
                    "text": "G-10c appended message",
                    "req_id": "g10c-send-request"
                }},
                "now_ms": CREATED_AT + 3
            }),
            json!({
                "kind": "PortReply",
                "value": {"correlation": send_persist.raw(), "outcome": {"variant": "Ok"}},
                "now_ms": CREATED_AT + 10
            }),
            json!({
                "kind": "PortReply",
                "value": {"correlation": refresh.raw(), "outcome": {
                    "variant": "Ok",
                    "rows": "21 durable message rows after append"
                }},
                "now_ms": CREATED_AT + 11
            }),
        ],
    })
}

/// 构造二十条真实 Go post 形状的首屏响应。
fn remote_page_body() -> TestResult<Bytes> {
    let posts = (1..=20).rev().map(remote_post).collect::<Vec<_>>();
    serde_json::to_vec(&json!({"status": "SUCCESS", "data": posts}))
        .map(Bytes::from)
        .map_err(|error| error.to_string())
}

/// 构造追加后倒序本地 rows，由 Helix 负责有界选择和头部淘汰。
pub(crate) fn local_rows_after_append() -> TestResult<Bytes> {
    let rows = (1..=21).rev().map(local_post).collect::<Vec<_>>();
    serde_json::to_vec(&rows)
        .map(Bytes::from)
        .map_err(|error| error.to_string())
}

/// 构造一条远端首屏 post。
fn remote_post(index: u32) -> Value {
    json!({
        "id": server_id(index),
        "temporaryId": temporary_id(index),
        "channelId": CHANNEL_ID,
        "userId": ACTOR_ID,
        "type": "TEXT",
        "message": format!("G-10c message {index}"),
        "simpleMessage": format!("G-10c message {index}"),
        "viewers": ["all"],
        "mentions": [],
        "eventSeq": index,
        "createAt": CREATED_AT + i64::from(index),
    })
}

/// 构造一条本地 message 表读回行。
pub(crate) fn local_post(index: u32) -> Value {
    json!({
        "id": server_id(index),
        "temporary_id": temporary_id(index),
        "channel_id": CHANNEL_ID,
        "user_id": ACTOR_ID,
        "type": "TEXT",
        "message": format!("G-10c message {index}"),
        "simple_message": format!("G-10c message {index}"),
        "send_status": "sent",
        "event_seq": index,
        "create_at": CREATED_AT + i64::from(index),
    })
}

/// 生成稳定 server id。
pub(crate) fn server_id(index: u32) -> String {
    format!("srvfix{index:020x}")
}

/// 生成稳定 temporary id。
fn temporary_id(index: u32) -> String {
    format!("tmpfix{index:020x}")
}

/// 将裸 Go body 包成 host 回灌的 ADR-007 HTTP envelope。
pub(crate) fn http_envelope(body: Bytes) -> TestResult<Bytes> {
    serde_json::to_vec(&json!({
        "status": 200,
        "headers": {},
        "body": base64_std(body.as_ref()),
    }))
    .map(Bytes::from)
    .map_err(|error| error.to_string())
}

/// 以标准表编码测试 HTTP envelope body。
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

/// G-10c 输出绝对有界窗口，追加 21 后淘汰头部 1。
#[test]
fn g10c_appends_into_full_window_and_evicts_oldest_row() -> TestResult<()> {
    let trace = run_success()?;
    assert_eq!(trace.events.len(), 1);
    let event = &trace.events[0];
    assert_eq!(event["event"], "im:timeline:update");
    assert_eq!(event["data"]["direction"], "append");
    assert_eq!(event["data"]["state"], "ready");
    let messages = event["data"]["messages"]
        .as_array()
        .ok_or_else(|| "G-10c messages must be an array".to_string())?;
    assert_eq!(messages.len(), 20);
    assert_eq!(messages[0]["id"], server_id(2));
    assert_eq!(messages[19]["id"], server_id(21));
    assert!(!messages.iter().any(|row| row["id"] == server_id(1)));
    Ok(())
}

/// G-10c 不向 consumer 泄漏 renderer mutation、selector 或 action capability。
#[test]
fn g10c_keeps_consumer_selection_outside_the_event_contract() -> TestResult<()> {
    let trace = run_success()?;
    let encoded = serde_json::to_string(&trace.events).map_err(|error| error.to_string())?;
    for forbidden in [
        "InteractionFrame",
        "Surface",
        "mutation",
        "operations",
        "selector",
        "actionCapabilities",
        "directive",
    ] {
        assert!(!encoded.contains(forbidden), "forbidden field {forbidden}");
    }
    Ok(())
}

/// Rust producer 在生成模式下写出 G-10c sender 与空 receiver canonical artifact。
#[test]
fn g10c_writes_reviewable_dual_persona_artifacts() -> TestResult<()> {
    let trace = run_success()?;
    let Some(output) = std::env::var_os("HELIX_GATE_ARTIFACT_DIR") else {
        return Ok(());
    };
    let bundle = artifact::gate_bundle(
        "G-10c",
        "message_v3_g10c",
        trace.ticks,
        trace.events,
        Vec::new(),
        json!([
            {"table": "post", "columns": ["id", "temporary_id", "channel_id", "user_id", "type", "message", "create_at"]}
        ]),
        json!([
            {"table": "message", "columns": ["id", "temporary_id", "channel_id", "user_id", "type", "message", "send_status", "create_at", "event_seq"]}
        ]),
        json!([
            {"table": "message", "columns": ["id", "temporary_id", "channel_id", "user_id", "type", "message", "send_status", "create_at", "event_seq"]}
        ]),
    );
    artifact::write_bundle(std::path::Path::new(&output), &bundle)
        .map_err(|error| error.to_string())
        .map(|_| ())
}
