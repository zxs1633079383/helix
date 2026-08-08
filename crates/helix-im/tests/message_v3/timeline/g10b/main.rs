// 回归锚: Spec 047/T052-T053，G-10b newer page durable window。

use bytes::Bytes;
use helix_core::effect::{Effect, StorageOp};
use helix_core::tick::{AppCommand, PortError, PortOutcome, ReplyBytes};
use helix_core::{Correlation, EffectSink, Module, Tick};
use helix_im::module::ImConfig;
use helix_im::query::LocalStoreMode;
use helix_im::state::ChannelId;
use helix_im::ImModule;
use serde_json::{json, Value};

#[path = "../../support/artifact.rs"]
mod artifact;

const CHANNEL_ID: &str = "chfixx0000000000000000001b";
const WINDOW_TOKEN: &str = "g10b-latest-window";
const ACTOR_ID: &str = "g10b-actor";
const OTHER_ID: &str = "g10b-other";
const CREATED_AT: i64 = 1_700_001_200_000;

type TestResult<T> = Result<T, String>;

/// 记录一个 persona 的真实 Tick 与 newer 终态。
struct PersonaTrace {
    events: Vec<Value>,
    ticks: Vec<Value>,
}

/// 通过公开 Module trait 驱动 G-10b newer 状态机。
struct Replay {
    module: ImModule,
    sink: EffectSink,
    now_ms: u64,
}

impl Replay {
    /// 创建带 Session store 和确定性 viewer 的回放器。
    fn new() -> TestResult<Self> {
        let mut module = ImModule::new_with_local_store(
            ImConfig {
                api_base_url: "http://g10b.invalid".to_string(),
                default_api_base_url: "http://g10b.invalid".to_string(),
                auth_user_id: ACTOR_ID.to_string(),
                company_id: "g10b-company".to_string(),
                ..ImConfig::default()
            },
            LocalStoreMode::Session,
        );
        let channel_id = ChannelId::from_str(CHANNEL_ID)
            .ok_or_else(|| "invalid G-10b channel fixture".to_string())?;
        module.register_channel(channel_id, 0);
        Ok(Self {
            module,
            sink: EffectSink::new(),
            now_ms: CREATED_AT as u64,
        })
    }

    /// 建立包含 anchor 的 attached latest window。
    fn attach_initial_window(&mut self) -> TestResult<()> {
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
            PortOutcome::Ok(ReplyBytes(http_envelope(latest_posts_body()?)?)),
            2,
        )?;
        let cache = self.only_corr("write", None)?;
        self.reply(cache, PortOutcome::Ok(ReplyBytes::default()), 3)?;
        // Cache barrier 只确认写入，必须再读 durable rows 才能结算 ready。
        let readback = self.only_corr("readback", None)?;
        self.reply(
            readback,
            PortOutcome::Ok(ReplyBytes(readback_row_bytes()?)),
            4,
        )?;
        if self.events()?.len() != 1 {
            return Err("G-10 prerequisite must emit one ready event".to_string());
        }
        Ok(())
    }

    /// 提交 canonical newer command 并返回 authority correlation。
    fn start_load(&mut self) -> TestResult<Correlation> {
        self.command(
            "im_load_newer_context",
            json!({
                "channel_id": CHANNEL_ID,
                "anchor_post_id": server_id(20),
                "pageSize": 20,
                "req_id": "g10b-request",
            }),
        )?;
        self.only_corr("http", Some("/posts/getPostsAfterIndex"))
    }

    /// 提交 JSON command 并执行真实 handle。
    fn command(&mut self, name: &'static str, payload: Value) -> TestResult<()> {
        let bytes = serde_json::to_vec(&payload).map_err(|error| error.to_string())?;
        self.step(Tick::Command(AppCommand::new(name, bytes)))
    }

    /// 回灌端口结果并推进确定性时钟。
    fn reply(&mut self, corr: Correlation, outcome: PortOutcome, offset_ms: u64) -> TestResult<()> {
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
    fn only_corr(&self, kind: &str, suffix: Option<&str>) -> TestResult<Correlation> {
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
                ("readback", Effect::Persist { ops, .. }) => ops.iter().any(|op| {
                    matches!(op, StorageOp::Get(spec) if spec.table == "message")
                        || matches!(op, StorageOp::Scan(spec) if spec.table == "message")
                }),
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

/// 回放完整 newer 成功链。
fn run_success() -> TestResult<PersonaTrace> {
    let mut replay = Replay::new()?;
    replay.attach_initial_window()?;
    let page_http = replay.start_load()?;
    replay.reply(
        page_http,
        PortOutcome::Ok(ReplyBytes(http_envelope(newer_page_body(false)?)?)),
        10,
    )?;
    if !replay.events()?.is_empty() {
        return Err("newer HTTP must not emit before message persist".to_string());
    }
    let write = replay.only_corr("write", None)?;
    replay.reply(write, PortOutcome::Ok(ReplyBytes::default()), 11)?;
    let readback = replay.only_corr("readback", None)?;
    replay.reply(
        readback,
        PortOutcome::Ok(ReplyBytes(readback_row_bytes()?)),
        12,
    )?;
    Ok(PersonaTrace {
        events: replay.events()?,
        ticks: vec![
            json!({
                "kind": "Command",
                "value": {"name": "im_load_newer_context", "payload": {
                    "channel_id": CHANNEL_ID,
                    "anchor_post_id": server_id(20),
                    "pageSize": 20,
                    "req_id": "g10b-request"
                }},
                "now_ms": CREATED_AT + 3
            }),
            json!({
                "kind": "PortReply",
                "value": {"correlation": page_http.raw(), "outcome": {
                    "variant": "Ok",
                    "body": {"posts": "20 newer post rows", "hasMore": false}
                }},
                "now_ms": CREATED_AT + 10
            }),
            json!({
                "kind": "PortReply",
                "value": {"correlation": readback.raw(), "outcome": {
                    "variant": "Ok",
                    "rows": "matching durable message row"
                }},
                "now_ms": CREATED_AT + 12
            }),
        ],
    })
}

/// 递归查找对象键，避免把 `reactions` 误判成 `actions`。
fn contains_key(value: &Value, expected: &str) -> bool {
    match value {
        Value::Object(object) => object
            .iter()
            .any(|(key, child)| key == expected || contains_key(child, expected)),
        Value::Array(values) => values.iter().any(|child| contains_key(child, expected)),
        _ => false,
    }
}

/// 断言 G-10b 只发布 durable newer page。
fn assert_newer_event(event: &Value, state: &str, has_newer: bool) -> TestResult<()> {
    assert_eq!(
        event["event"], "im:read:result",
        "unexpected G-10b event: {event}"
    );
    assert_eq!(event["data"]["req_id"], "g10b-request");
    if state == "failed" {
        assert!(event["data"]["error"].is_string());
        return Ok(());
    }
    assert_eq!(state, "ready");
    let body = &event["data"]["body"];
    assert_eq!(body["reqId"], "g10b-request");
    assert_eq!(body["channelId"], CHANNEL_ID);
    assert_eq!(body["operation"], "newer");
    assert_eq!(body["hasNewer"], has_newer);
    assert!(body["messages"].as_array().is_some());
    for forbidden in [
        "InteractionFrame",
        "Surface",
        "mutation",
        "actionCapabilities",
        "actions",
        "directive",
    ] {
        assert!(
            !contains_key(event, forbidden),
            "forbidden field {forbidden}"
        );
    }
    Ok(())
}

/// G-10b 成功链在 durable barrier 后发布耗尽的 newer page。
#[test]
fn g10b_success_appends_newer_page_after_durable_readback() -> TestResult<()> {
    let trace = run_success()?;
    assert_eq!(trace.events.len(), 1);
    assert_newer_event(&trace.events[0], "ready", false)?;
    assert_eq!(
        trace.events[0]["data"]["body"]["messages"]
            .as_array()
            .map(Vec::len),
        Some(1)
    );
    assert_eq!(
        trace.events[0]["data"]["body"]["messages"][0]["id"],
        server_id(40)
    );
    Ok(())
}

/// G-10b authority 与持久化失败都不得泄漏 ready。
#[test]
fn g10b_failures_emit_empty_failed_page() -> TestResult<()> {
    let mut http_failure = Replay::new()?;
    http_failure.attach_initial_window()?;
    let http = http_failure.start_load()?;
    http_failure.reply(http, PortOutcome::Err(PortError::Http(500)), 10)?;
    let events = http_failure.events()?;
    assert_eq!(events.len(), 1);
    assert_newer_event(&events[0], "failed", false)?;

    let mut persist_failure = Replay::new()?;
    persist_failure.attach_initial_window()?;
    let http = persist_failure.start_load()?;
    persist_failure.reply(
        http,
        PortOutcome::Ok(ReplyBytes(http_envelope(newer_page_body(true)?)?)),
        10,
    )?;
    let write = persist_failure.only_corr("write", None)?;
    persist_failure.reply(write, PortOutcome::Err(PortError::Storage(500)), 11)?;
    let events = persist_failure.events()?;
    assert_eq!(events.len(), 1);
    assert_newer_event(&events[0], "failed", true)?;
    Ok(())
}

/// G-10b matching correlation 消费后重复回报保持 no-op。
#[test]
fn g10b_duplicate_readback_is_noop() -> TestResult<()> {
    let mut replay = Replay::new()?;
    replay.attach_initial_window()?;
    let http = replay.start_load()?;
    replay.reply(
        http,
        PortOutcome::Ok(ReplyBytes(http_envelope(newer_page_body(false)?)?)),
        10,
    )?;
    let write = replay.only_corr("write", None)?;
    replay.reply(write, PortOutcome::Ok(ReplyBytes::default()), 11)?;
    let readback = replay.only_corr("readback", None)?;
    replay.reply(
        readback,
        PortOutcome::Ok(ReplyBytes(readback_row_bytes()?)),
        12,
    )?;
    assert_eq!(replay.events()?.len(), 1);
    replay.reply(
        readback,
        PortOutcome::Ok(ReplyBytes(readback_row_bytes()?)),
        13,
    )?;
    assert!(replay.sink.as_slice().is_empty());
    Ok(())
}

/// Rust producer 写出 persona-local G-10b canonical artifact。
#[test]
fn g10b_writes_reviewable_persona_artifacts() -> TestResult<()> {
    let sender = run_success()?;
    let Some(output) = std::env::var_os("HELIX_GATE_ARTIFACT_DIR") else {
        return Ok(());
    };
    let bundle = artifact::gate_bundle(
        "G-10b",
        "message_v3_g10b",
        sender.ticks,
        sender.events,
        Vec::new(),
        json!([
            {"table": "post", "columns": ["id", "temporary_id", "channel_id", "user_id", "type", "message", "props", "event_seq", "create_at"]}
        ]),
        json!([
            {"table": "message", "columns": ["temporary_id", "id", "channel_id", "user_id", "type", "message", "props", "event_seq", "send_status", "create_at", "update_at"]}
        ]),
        json!([]),
    );
    artifact::write_bundle(std::path::Path::new(&output), &bundle)
        .map_err(|error| error.to_string())?;
    Ok(())
}

/// 构造 recent 20 条 Go post 响应。
fn latest_posts_body() -> TestResult<Bytes> {
    let posts = (1..=20).rev().map(remote_post).collect::<Vec<_>>();
    serde_json::to_vec(&json!({"status": "SUCCESS", "data": posts}))
        .map(Bytes::from)
        .map_err(|error| error.to_string())
}

/// 构造包含 anchor 与 20 条更新 post 的 authority page。
fn newer_page_body(has_more: bool) -> TestResult<Bytes> {
    let posts = (20..=40).map(remote_post).collect::<Vec<_>>();
    serde_json::to_vec(&json!({
        "status": "SUCCESS",
        "data": {"posts": posts, "hasMore": has_more}
    }))
    .map(Bytes::from)
    .map_err(|error| error.to_string())
}

/// 构造 newer 尾行的 durable readback。
fn readback_row_bytes() -> TestResult<Bytes> {
    serde_json::to_vec(&vec![json!({
        "id": server_id(40),
        "temporary_id": "g10b-temporary-40",
        "channel_id": CHANNEL_ID,
        "user_id": ACTOR_ID,
        "type": "TEXT",
        "message": "g10b-message-40",
        "simple_message": "g10b-message-40",
        "create_at": CREATED_AT + 40,
        "send_status": "sent",
        "event_seq": 40,
    })])
    .map(Bytes::from)
    .map_err(|error| error.to_string())
}

/// 构造 viewer-visible Go wire Post。
fn remote_post(index: u32) -> Value {
    json!({
        "id": server_id(index),
        "temporaryId": format!("g10b-temporary-{index}"),
        "channelId": CHANNEL_ID,
        "userId": if index % 2 == 0 { ACTOR_ID } else { OTHER_ID },
        "type": "TEXT",
        "message": format!("g10b-message-{index}"),
        "simpleMessage": format!("g10b-message-{index}"),
        "viewers": ["all"],
        "mentions": [],
        "eventSeq": index,
        "createAt": CREATED_AT + i64::from(index),
    })
}

/// 生成稳定 26 字符 server id。
fn server_id(index: u32) -> String {
    format!("srvfix{index:020x}")
}

/// 将裸 Go body 包成 ADR-007 HTTP envelope。
fn http_envelope(body: Bytes) -> TestResult<Bytes> {
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
