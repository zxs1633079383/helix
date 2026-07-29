// 回归锚: Spec 047/T062-T063，G-12 回复列表与分支。

use bytes::Bytes;
use helix_core::effect::Effect;
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
const ROOT_ID: &str = "srvfix0000000000000000001c";
const FIRST_REPLY_ID: &str = "srvfix0000000000000000002c";
const SECOND_REPLY_ID: &str = "srvfix0000000000000000003c";
const BRANCH_REPLY_ID: &str = "srvfix0000000000000000004c";
const CREATED_AT: i64 = 1_700_001_300_000;

type TestResult<T> = Result<T, String>;

/// 保存 snapshot 与 append 两个真实终态事件。
struct PersonaTrace {
    events: Vec<Value>,
    ticks: Vec<Value>,
}

/// 通过公开 `ImModule::handle` 驱动回复读链路。
struct Replay {
    module: ImModule,
    sink: EffectSink,
}

impl Replay {
    /// 创建固定身份和 session store 的 G-12 回放器。
    fn new() -> TestResult<Self> {
        let mut module = ImModule::new_with_local_store(
            ImConfig {
                api_base_url: "http://g12.invalid".to_string(),
                default_api_base_url: "http://g12.invalid".to_string(),
                auth_user_id: "g12-sender".to_string(),
                company_id: "g12-company".to_string(),
                ..ImConfig::default()
            },
            LocalStoreMode::Session,
        );
        let channel_id =
            ChannelId::from_str(CHANNEL_ID).ok_or_else(|| "invalid channel fixture".to_string())?;
        module.register_channel(channel_id, 0);
        Ok(Self {
            module,
            sink: EffectSink::new(),
        })
    }

    /// 执行单步并覆盖当前 Effect batch。
    fn step(&mut self, tick: Tick, now_ms: u64) -> TestResult<()> {
        self.sink.clear();
        self.module
            .handle(&tick, now_ms, &mut self.sink)
            .map_err(|error| error.to_string())
    }

    /// 发起回复读 command 并返回唯一 HTTP correlation。
    fn command(&mut self, name: &'static str, payload: Value) -> TestResult<Correlation> {
        let bytes = serde_json::to_vec(&payload).map_err(|error| error.to_string())?;
        self.step(
            Tick::Command(AppCommand::new(name, bytes)),
            CREATED_AT as u64,
        )?;
        self.sink
            .as_slice()
            .iter()
            .find_map(|effect| match effect {
                Effect::Http { corr, .. } => Some(*corr),
                _ => None,
            })
            .ok_or_else(|| format!("{name} did not emit HTTP"))
    }

    /// 回灌 HTTP authority。
    fn reply(&mut self, corr: Correlation, body: Value, now_ms: u64) -> TestResult<()> {
        let payload = serde_json::to_vec(&body).map_err(|error| error.to_string())?;
        let envelope = http_envelope(&payload)?;
        self.step(
            Tick::PortReply {
                corr,
                outcome: PortOutcome::Ok(ReplyBytes(Bytes::from(envelope))),
            },
            now_ms,
        )
    }

    /// 解码当前 batch 中的 MessageV3 事件。
    fn events(&self) -> TestResult<Vec<Value>> {
        self.sink
            .as_slice()
            .iter()
            .filter_map(|effect| match effect {
                Effect::Emit { event } => Some(
                    serde_json::from_slice(event.0.as_ref())
                        .map_err(|error| error.to_string()),
                ),
                _ => None,
            })
            .collect()
    }
}

/// 回放 snapshot 后的 attached branch append。
fn run_success() -> TestResult<PersonaTrace> {
    let mut replay = Replay::new()?;
    let snapshot_corr = replay.command(
        "im_get_replies",
        json!({
            "reply_id": ROOT_ID,
            "req_id": "g12-snapshot",
            "channel_id": CHANNEL_ID,
            "root_message_id": ROOT_ID,
            "page_number": 1,
            "page_size": 20
        }),
    )?;
    replay.reply(snapshot_corr, snapshot_body(), CREATED_AT as u64 + 1)?;
    let mut events = replay.events()?;
    let branch_corr = replay.command(
        "im_get_reply_branch",
        json!({
            "reply_first_level_id": FIRST_REPLY_ID,
            "req_id": "g12-branch",
            "channel_id": CHANNEL_ID,
            "root_message_id": ROOT_ID,
            "page_number": 1,
            "page_size": 20
        }),
    )?;
    replay.reply(branch_corr, branch_body(), CREATED_AT as u64 + 2)?;
    events.extend(replay.events()?);
    Ok(PersonaTrace {
        events,
        ticks: vec![
            json!({"kind":"Command","value":{"name":"im_get_replies","payload":{"channel_id":CHANNEL_ID,"reply_id":ROOT_ID}},"now_ms":CREATED_AT}),
            json!({"kind":"PortReply","value":{"correlation":snapshot_corr.raw(),"outcome":{"variant":"Ok"}},"now_ms":CREATED_AT + 1}),
            json!({"kind":"Command","value":{"name":"im_get_reply_branch","payload":{"channel_id":CHANNEL_ID,"reply_first_level_id":FIRST_REPLY_ID}},"now_ms":CREATED_AT + 1}),
            json!({"kind":"PortReply","value":{"correlation":branch_corr.raw(),"outcome":{"variant":"Ok"}},"now_ms":CREATED_AT + 2}),
        ],
    })
}

/// 构造服务端 snapshot authority。
fn snapshot_body() -> Value {
    json!({
        "rootPost": post(ROOT_ID, "root", "", ""),
        "replies": [
            post(FIRST_REPLY_ID, "first", ROOT_ID, FIRST_REPLY_ID),
            post(SECOND_REPLY_ID, "second", ROOT_ID, SECOND_REPLY_ID)
        ],
        "hasMore": false
    })
}

/// 构造服务端 branch authority。
fn branch_body() -> Value {
    json!({
        "data": [post(BRANCH_REPLY_ID, "branch", ROOT_ID, FIRST_REPLY_ID)],
        "hasMore": false
    })
}

/// 构造一条远端回复事实。
fn post(id: &str, message: &str, root: &str, first_level: &str) -> Value {
    json!({
        "id": id,
        "channelId": CHANNEL_ID,
        "userId": "g12-sender",
        "type": "TEXT",
        "message": message,
        "simpleMessage": message,
        "replyId": root,
        "replyRootId": root,
        "replyFirstLevelId": first_level,
        "viewers": ["all"],
        "mentions": [],
        "createAt": CREATED_AT
    })
}

/// 把业务 body 包装为 host HTTP reply envelope。
fn http_envelope(body: &[u8]) -> TestResult<Vec<u8>> {
    serde_json::to_vec(&json!({
        "status": 200,
        "headers": {},
        "body": base64_std(body)
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

/// G-12 snapshot 与 branch 都发布真实回复绝对态。
#[test]
fn g12_emits_snapshot_and_branch_events() -> TestResult<()> {
    let trace = run_success()?;
    assert_eq!(trace.events.len(), 2);
    assert_eq!(trace.events[0]["event"], "im:timeline:thread");
    assert_eq!(trace.events[0]["data"]["mode"], "snapshot");
    assert_eq!(trace.events[0]["data"]["rootMessageId"], ROOT_ID);
    assert_eq!(trace.events[0]["data"]["replyCount"], 2);
    assert_eq!(trace.events[1]["event"], "im:timeline:thread");
    assert_eq!(trace.events[1]["data"]["mode"], "append");
    assert_eq!(trace.events[1]["data"]["replyIds"][0], BRANCH_REPLY_ID);
    Ok(())
}

/// G-12 事件不泄露 Frame、Surface 或 renderer action。
#[test]
fn g12_has_no_renderer_shell_fields() -> TestResult<()> {
    let encoded = serde_json::to_string(&run_success()?.events)
        .map_err(|error| error.to_string())?;
    for forbidden in [
        "InteractionFrame",
        "Surface",
        "mutation",
        "directive",
        "actionCapabilities",
    ] {
        assert!(!encoded.contains(forbidden), "forbidden field {forbidden}");
    }
    Ok(())
}

/// Rust producer 在生成模式下写出 G-12 双视角 canonical artifact。
#[test]
fn artifact_test() -> TestResult<()> {
    let trace = run_success()?;
    let Some(output) = std::env::var_os("HELIX_GATE_ARTIFACT_DIR") else {
        return Ok(());
    };
    let bundle = artifact::gate_bundle(
        "G-12",
        "message_v3_g12",
        trace.ticks,
        trace.events,
        Vec::new(),
        json!([{"table":"post","columns":["id","channel_id","user_id","reply_id","reply_first_level_id","message","create_at"]}]),
        json!([{"table":"message","columns":["id","channel_id","user_id","reply_id","reply_first_level_id","message","create_at"]}]),
        json!([{"table":"message","columns":["id","channel_id","user_id","reply_id","reply_first_level_id","message","create_at"]}]),
    );
    artifact::write_bundle(std::path::Path::new(&output), &bundle)
        .map_err(|error| error.to_string())
        .map(|_| ())
}
