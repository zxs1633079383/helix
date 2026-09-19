// 回归锚: Spec 047/T048-T049，G-10 local-first timeline ready 绝对态。

use bytes::Bytes;
use helix_core::effect::{Effect, SqlValue, StorageOp, UpsertSpec};
use helix_core::tick::{AppCommand, PortError, PortOutcome, ReplyBytes};
use helix_core::{Correlation, EffectSink, Module, Tick};
use helix_im::module::ImConfig;
use helix_im::query::LocalStoreMode;
use helix_im::state::ChannelId;
use helix_im::ImModule;
use serde_json::{json, Value};

#[path = "../../support/artifact.rs"]
mod artifact;

const CHANNEL_ID: &str = "chfixx00000000000000000010";
const WINDOW_TOKEN: &str = "g10-latest-window";
const SENDER_ID: &str = "g10-sender";
const RECEIVER_ID: &str = "g10-receiver";
const COMPANY_ID: &str = "g10-company";
const CREATED_AT: i64 = 1_700_001_000_000;

type TestResult<T> = Result<T, String>;

/// 保存一个 persona 的真实 Tick 与 timeline 终态。
struct PersonaTrace {
    events: Vec<Value>,
    ticks: Vec<Value>,
}

/// 通过公开 Module trait 驱动 G-10 local-first 状态机。
struct Replay {
    module: ImModule,
    sink: EffectSink,
    now_ms: u64,
}

impl Replay {
    /// 创建带 Session store、确定性身份和已注册 channel 的回放器。
    fn new(viewer_user_id: &str) -> TestResult<Self> {
        let mut module = ImModule::new_with_local_store(
            ImConfig {
                api_base_url: "http://g10.invalid".to_string(),
                default_api_base_url: "http://g10.invalid".to_string(),
                auth_user_id: viewer_user_id.to_string(),
                company_id: COMPANY_ID.to_string(),
                ..ImConfig::default()
            },
            LocalStoreMode::Session,
        );
        let channel_id = ChannelId::from_str(CHANNEL_ID)
            .ok_or_else(|| "invalid G-10 channel fixture".to_string())?;
        module.register_channel(channel_id, 0);
        // 通过真实 PersistOk 建立 render_scope 所需的 channel{id,team_id,user_id}；不从消息作者推断归属。
        let mut sink = EffectSink::new();
        let row = vec![
            ("id".to_string(), SqlValue::Text(CHANNEL_ID.to_string())),
            (
                "team_id".to_string(),
                SqlValue::Text(COMPANY_ID.to_string()),
            ),
            (
                "user_id".to_string(),
                SqlValue::Text(viewer_user_id.to_string()),
            ),
        ];
        let corr = module.queue_message_v3_commit(
            vec![StorageOp::BatchUpsert(UpsertSpec::new(
                "channel",
                vec![row],
                Some("id"),
            ))],
            Vec::new(),
            &mut sink,
        );
        sink.clear();
        module
            .handle(
                &Tick::PortReply {
                    corr,
                    outcome: PortOutcome::Ok(ReplyBytes::default()),
                },
                CREATED_AT as u64,
                &mut sink,
            )
            .map_err(|error| error.to_string())?;
        sink.clear();
        Ok(Self {
            module,
            sink,
            now_ms: CREATED_AT as u64,
        })
    }

    /// 提交 canonical G-10 query 并返回本地 Scan correlation。
    fn start_query(&mut self) -> TestResult<Correlation> {
        let payload = serde_json::to_vec(&json!({
            "channel_id": CHANNEL_ID,
            "limit": 20,
            "windowToken": WINDOW_TOKEN,
        }))
        .map_err(|error| error.to_string())?;
        self.step(Tick::Command(AppCommand::new(
            "im_query_messages_by_channel",
            payload,
        )))?;
        self.only_corr("persist")
    }

    /// 回灌端口结果并推进确定性时钟。
    fn reply(&mut self, corr: Correlation, outcome: PortOutcome, offset_ms: u64) -> TestResult<()> {
        self.now_ms = CREATED_AT as u64 + offset_ms;
        self.step(Tick::PortReply { corr, outcome })
    }

    /// 通过真实 `ImModule::handle` 执行一步并保留该步 Effect。
    fn step(&mut self, tick: Tick) -> TestResult<()> {
        self.sink.clear();
        self.module
            .handle(&tick, self.now_ms, &mut self.sink)
            .map_err(|error| error.to_string())
    }

    /// 读取当前 batch 中指定类型的唯一 correlation。
    fn only_corr(&self, kind: &str) -> TestResult<Correlation> {
        let mut correlations =
            self.sink
                .as_slice()
                .iter()
                .filter_map(|effect| match (kind, effect) {
                    ("persist", Effect::Persist { corr, .. }) => Some(*corr),
                    ("http", Effect::Http { corr, .. }) => Some(*corr),
                    _ => None,
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

    /// 判断当前 batch 是否含 message cache 写入。
    fn has_message_cache_write(&self) -> bool {
        self.sink.as_slice().iter().any(|effect| match effect {
            Effect::Persist { ops, .. } => ops
                .iter()
                .any(|op| matches!(op, StorageOp::BatchUpsert(spec) if spec.table == "message")),
            _ => false,
        })
    }

    /// 统计当前 batch 的 HTTP Effect。
    fn http_count(&self) -> usize {
        self.sink
            .as_slice()
            .iter()
            .filter(|effect| matches!(effect, Effect::Http { .. }))
            .count()
    }
}

/// 构造 20 条真实 Go post 形状，保持服务端倒序返回。
fn latest_posts_body(marker: &str) -> TestResult<Bytes> {
    let posts = (1..=20)
        .rev()
        .map(|index| remote_post(index, marker))
        .collect::<Vec<_>>();
    serde_json::to_vec(&json!({"status": "SUCCESS", "data": posts}))
        .map(Bytes::from)
        .map_err(|error| error.to_string())
}

/// 构造与 SQLite message Scan 等价的倒序本地 rows。
fn local_rows_bytes(marker: &str) -> TestResult<Bytes> {
    let rows = (1..=20)
        .rev()
        .map(|index| {
            json!({
                "id": server_id(index),
                "temporary_id": format!("g10-temporary-{index}"),
                "channel_id": CHANNEL_ID,
                "user_id": if index % 2 == 0 { SENDER_ID } else { RECEIVER_ID },
                "type": "TEXT",
                "message": format!("{marker}-{index}"),
                "simple_message": format!("{marker}-{index}"),
                "create_at": CREATED_AT + i64::from(index),
                "send_status": "sent",
                "event_seq": index,
            })
        })
        .collect::<Vec<_>>();
    serde_json::to_vec(&rows)
        .map(Bytes::from)
        .map_err(|error| error.to_string())
}

/// 构造一条 viewer-visible remote post。
fn remote_post(index: u32, marker: &str) -> Value {
    json!({
        "id": server_id(index),
        "temporaryId": format!("g10-temporary-{index}"),
        "channelId": CHANNEL_ID,
        "userId": if index % 2 == 0 { SENDER_ID } else { RECEIVER_ID },
        "type": "TEXT",
        "message": format!("{marker}-{index}"),
        "simpleMessage": format!("{marker}-{index}"),
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

/// 将裸 Go body 包成 host 回灌的 ADR-007 HTTP envelope。
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

/// 回放远端首屏、cache barrier 与同一 persona 的本地恢复。
fn run_ready(viewer_user_id: &str, marker: &str) -> TestResult<PersonaTrace> {
    let mut replay = Replay::new(viewer_user_id)?;
    let scan = replay.start_query()?;
    replay.reply(
        scan,
        PortOutcome::Ok(ReplyBytes(Bytes::from_static(b"[]"))),
        1,
    )?;
    let http = replay.only_corr("http")?;
    let body = http_envelope(latest_posts_body(marker)?)?;
    replay.reply(http, PortOutcome::Ok(ReplyBytes(body)), 2)?;
    if !replay.has_message_cache_write() || !replay.events()?.is_empty() {
        return Err("G-10 must wait for cache PersistOk".to_string());
    }
    let cache = replay.only_corr("persist")?;
    replay.reply(cache, PortOutcome::Ok(ReplyBytes::default()), 3)?;
    // Cache barrier 只确认写入，随后必须读取 durable message rows 才能发布 ready。
    let readback = replay.only_corr("persist")?;
    replay.reply(
        readback,
        PortOutcome::Ok(ReplyBytes(local_rows_bytes(marker)?)),
        4,
    )?;
    let events = replay.events()?;
    Ok(PersonaTrace {
        events,
        ticks: vec![
            json!({
                "kind": "Command",
                "value": {"name": "im_query_messages_by_channel", "payload": {
                    "channel_id": CHANNEL_ID,
                    "limit": 20,
                    "windowToken": WINDOW_TOKEN
                }},
                "now_ms": CREATED_AT
            }),
            json!({
                "kind": "PortReply",
                "value": {"correlation": http.raw(), "outcome": {
                    "variant": "Ok",
                    "body": {"status": "SUCCESS", "data": "20 remote post rows"}
                }},
                "now_ms": CREATED_AT + 2
            }),
            json!({
                "kind": "PortReply",
                "value": {"correlation": cache.raw(), "outcome": {"variant": "Ok"}},
                "now_ms": CREATED_AT + 3
            }),
            json!({
                "kind": "PortReply",
                "value": {"correlation": readback.raw(), "outcome": {
                    "variant": "Ok",
                    "body": {"rows": "20 durable message rows"}
                }},
                "now_ms": CREATED_AT + 4
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

/// 断言 G-10 事件只携窗口状态和实际 post 行。
fn assert_ready_event(event: &Value, marker: &str) -> TestResult<()> {
    assert_eq!(event["event"], "im:timeline:ready");
    assert_eq!(event["data"]["channelId"], CHANNEL_ID);
    assert_eq!(event["data"]["windowToken"], WINDOW_TOKEN);
    assert_eq!(event["data"]["state"], "ready");
    assert_eq!(event["data"]["hasOlder"], true);
    assert_eq!(event["data"]["hasNewer"], false);
    let messages = event["data"]["messages"]
        .as_array()
        .ok_or_else(|| "G-10 messages must be an array".to_string())?;
    assert_eq!(messages.len(), 20);
    assert_eq!(messages[19]["message"], format!("{marker}-20"));
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

/// G-10 远端首屏必须通过 cache barrier 后发布。
#[test]
fn g10_remote_ready_waits_for_cache_persist() -> TestResult<()> {
    let trace = run_ready(SENDER_ID, "g10-ready")?;
    assert_eq!(trace.events.len(), 1);
    assert_ready_event(&trace.events[0], "g10-ready")
}

/// G-10 HTTP 失败只发布 failed 绝对态且不伪造空成功。
#[test]
fn g10_http_failure_emits_failed_window() -> TestResult<()> {
    let mut replay = Replay::new(SENDER_ID)?;
    let scan = replay.start_query()?;
    replay.reply(
        scan,
        PortOutcome::Ok(ReplyBytes(Bytes::from_static(b"[]"))),
        1,
    )?;
    let http = replay.only_corr("http")?;
    replay.reply(http, PortOutcome::Err(PortError::Http(500)), 2)?;
    let events = replay.events()?;
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["event"], "im:timeline:ready");
    assert_eq!(events[0]["data"]["state"], "failed");
    assert_eq!(events[0]["data"]["messages"], json!([]));
    Ok(())
}

/// G-10 durable coverage 可直接从本地 message rows 重建 ready 终态。
#[test]
fn g10_local_recovery_avoids_remote_http() -> TestResult<()> {
    let mut replay = Replay::new(SENDER_ID)?;
    let initial_scan = replay.start_query()?;
    replay.reply(
        initial_scan,
        PortOutcome::Ok(ReplyBytes(Bytes::from_static(b"[]"))),
        1,
    )?;
    let initial_http = replay.only_corr("http")?;
    replay.reply(
        initial_http,
        PortOutcome::Ok(ReplyBytes(http_envelope(latest_posts_body(
            "g10-recovery",
        )?)?)),
        2,
    )?;
    let cache = replay.only_corr("persist")?;
    replay.reply(cache, PortOutcome::Ok(ReplyBytes::default()), 3)?;
    let readback = replay.only_corr("persist")?;
    replay.reply(
        readback,
        PortOutcome::Ok(ReplyBytes(local_rows_bytes("g10-recovery")?)),
        4,
    )?;

    let recovery_scan = replay.start_query()?;
    replay.reply(
        recovery_scan,
        PortOutcome::Ok(ReplyBytes(local_rows_bytes("g10-recovery")?)),
        4,
    )?;
    assert_eq!(replay.http_count(), 0);
    let events = replay.events()?;
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["event"], "im:timeline:update");
    assert_eq!(events[0]["data"]["channelId"], CHANNEL_ID);
    assert_eq!(events[0]["data"]["windowToken"], WINDOW_TOKEN);
    assert_eq!(events[0]["data"]["direction"], "append");
    assert_eq!(events[0]["data"]["state"], "ready");
    assert_eq!(
        events[0]["data"]["messages"].as_array().map(Vec::len),
        Some(20)
    );
    assert_eq!(
        events[0]["data"]["messages"][19]["message"],
        "g10-recovery-20"
    );
    Ok(())
}

/// G-10 两个 persona 分别从自身状态机得到独立的 viewer-local 窗口。
#[test]
fn g10_dual_persona_windows_are_independent() -> TestResult<()> {
    let sender = run_ready(SENDER_ID, "g10-dual")?;
    let receiver = run_ready(RECEIVER_ID, "g10-dual")?;
    assert_ready_event(&sender.events[0], "g10-dual")?;
    assert_ready_event(&receiver.events[0], "g10-dual")?;
    assert_ne!(sender.events, receiver.events);
    assert_eq!(sender.events[0]["data"]["messages"][0]["isSelf"], false);
    assert_eq!(sender.events[0]["data"]["messages"][1]["isSelf"], true);
    assert_eq!(receiver.events[0]["data"]["messages"][0]["isSelf"], true);
    assert_eq!(receiver.events[0]["data"]["messages"][1]["isSelf"], false);
    Ok(())
}

/// Rust producer 在生成模式下写出 G-10 双 persona canonical artifact。
#[test]
fn g10_writes_reviewable_dual_persona_artifacts() -> TestResult<()> {
    let sender = run_ready(SENDER_ID, "g10-artifact")?;
    let receiver = run_ready(RECEIVER_ID, "g10-artifact")?;
    let Some(output) = std::env::var_os("HELIX_GATE_ARTIFACT_DIR") else {
        return Ok(());
    };
    let mut ticks = sender.ticks;
    ticks.extend(receiver.ticks);
    let bundle = artifact::gate_bundle(
        "G-10",
        "message_v3_g10",
        ticks,
        sender.events,
        receiver.events,
        json!([
            {"table": "post", "columns": ["id", "temporary_id", "channel_id", "user_id", "type", "message", "props", "viewers", "mentions", "event_seq", "create_at"]}
        ]),
        json!([
            {"table": "message", "columns": ["temporary_id", "id", "channel_id", "user_id", "type", "message", "props", "viewers", "mentions", "event_seq", "send_status", "create_at", "update_at"]}
        ]),
        json!([
            {"table": "message", "columns": ["temporary_id", "id", "channel_id", "user_id", "type", "message", "props", "viewers", "mentions", "event_seq", "send_status", "create_at", "update_at"]}
        ]),
    );
    artifact::write_bundle(std::path::Path::new(&output), &bundle)
        .map_err(|error| error.to_string())?;
    Ok(())
}
