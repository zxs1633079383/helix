// 回归锚: Spec 047/T064-T065，G-13 会话已读绝对态。

use bytes::Bytes;
use helix_core::effect::{Effect, SqlValue, StorageOp};
use helix_core::tick::{AppCommand, InboundBytes, PortError, PortOutcome, ReplyBytes};
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

    /// 回灌查询第二阶段的 viewer 成员绝对态，终态必须等待真实 ScopedScan。
    fn reply_dialog_members(
        &mut self,
        unread: i64,
        mention: i64,
        urgent: i64,
        now_ms: u64,
    ) -> TestResult<Correlation> {
        assert!(
            self.events()?.is_empty(),
            "member readback must precede Result"
        );
        let [Effect::Persist { corr, ops }] = self.sink.as_slice() else {
            return Err("channel read must produce one member scan".into());
        };
        let [StorageOp::ScopedScan(spec)] = ops.as_slice() else {
            return Err("member read must be channel-scoped".into());
        };
        assert_eq!(spec.table, "channel_member");
        assert_eq!(spec.scope_values.len(), 1);
        assert!(matches!(&spec.scope_values[0], SqlValue::Text(id) if id == CHANNEL_ID));
        let corr = *corr;
        self.reply(
            corr,
            json!([{
                "channel_id": CHANNEL_ID, "user_id": ACTOR_ID,
                "channel_is_top": 0,
                "unread_count": unread, "mention_count": mention,
                "urgent_mention_count": urgent,
                "unread_post_id": if unread > 0 { "g13-unread-anchor" } else { "" },
                "last_read_seq": if unread == 0 { 41 } else { 38 },
                "projection_revision": 9, "effect_id": "g13-member-effect-9"
            }])
            .to_string()
            .into_bytes(),
            now_ms,
        )?;
        Ok(corr)
    }

    /// 回灌失败结果，验证失败不释放旧绝对投影。
    fn reply_error(&mut self, corr: Correlation, error: PortError, now_ms: u64) -> TestResult<()> {
        self.step(
            Tick::PortReply {
                corr,
                outcome: PortOutcome::Err(error),
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
    let initial_member_corr = replay.reply_dialog_members(3, 1, 1, CREATED_AT as u64 + 1)?;
    let initial = replay.events()?;

    let view_corr = replay.command_corr(
        "im_channels_view",
        json!({"channels":[{"id":CHANNEL_ID,"isRoot":true}]}),
    )?;
    replay.reply(view_corr, http_success()?, CREATED_AT as u64 + 2)?;
    if !replay.events()?.is_empty() {
        return Err("channels/view HTTP admission 不得直接发布业务终态".to_string());
    }
    let persist_corr = replay.ingest_read()?;
    replay.reply(persist_corr, Vec::new(), CREATED_AT as u64 + 3)?;
    let terminal = replay.events()?;
    let terminal_count = terminal
        .iter()
        .filter(|event| event["event"] == "im:channel:update")
        .count();
    if terminal_count != 1 {
        return Err(format!(
            "update_channel 应恰好发布一次 absolute terminal，实际 {terminal_count}"
        ));
    }
    replay.reply(persist_corr, Vec::new(), CREATED_AT as u64 + 4)?;
    if !replay.events()?.is_empty() {
        return Err("迟到/重复 PersistReply 不得重复发布 terminal".to_string());
    }

    let mut recovered = Replay::new();
    let recovery_corr = recovered.command_corr("im_query_dialog_list", json!({}))?;
    recovered.reply(
        recovery_corr,
        serde_json::to_vec(&vec![dialog_row(0, 0, 0)]).map_err(|error| error.to_string())?,
        CREATED_AT as u64 + 4,
    )?;
    let recovery_member_corr = recovered.reply_dialog_members(0, 0, 0, CREATED_AT as u64 + 4)?;
    let recovery = recovered.events()?;
    let mut events = initial;
    events.extend(terminal);
    events.extend(recovery);
    Ok(PersonaTrace {
        events,
        ticks: vec![
            json!({"kind":"Command","value":{"name":"im_query_dialog_list"},"now_ms":CREATED_AT}),
            json!({"kind":"PortReply","value":{"correlation":query_corr.raw(),"outcome":{"variant":"Ok"}},"now_ms":CREATED_AT + 1}),
            json!({"kind":"PortReply","value":{"correlation":initial_member_corr.raw(),"outcome":{"variant":"Ok"}},"now_ms":CREATED_AT + 1}),
            json!({"kind":"Command","value":{"name":"im_channels_view","payload":{"channels":[{"id":CHANNEL_ID,"isRoot":true}]}},"now_ms":CREATED_AT + 1}),
            json!({"kind":"Inbound","value":{"action":"update_channel","event_seq":42},"now_ms":CREATED_AT + 2}),
            json!({"kind":"PortReply","value":{"correlation":persist_corr.raw(),"outcome":{"variant":"Ok"}},"now_ms":CREATED_AT + 3}),
            json!({"kind":"Command","value":{"name":"im_query_dialog_list","recovery":true},"now_ms":CREATED_AT + 3}),
            json!({"kind":"PortReply","value":{"correlation":recovery_corr.raw(),"outcome":{"variant":"Ok"}},"now_ms":CREATED_AT + 4}),
            json!({"kind":"PortReply","value":{"correlation":recovery_member_corr.raw(),"outcome":{"variant":"Ok"}},"now_ms":CREATED_AT + 4}),
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
        "user_id": ACTOR_ID,
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

/// G-13 初始未读与冷恢复走单一 typed Result，在线已读仍发布权威绝对事件。
#[test]
fn g13_emits_initial_terminal_and_recovery_states() -> TestResult<()> {
    let trace = run_success()?;
    assert_eq!(trace.events.len(), 4);
    assert_eq!(trace.events[0]["event"], "im:read:result");
    assert_eq!(
        trace.events[0]["data"]["body"]["items"][0]["unreadCount"],
        3
    );
    assert_eq!(
        trace.events[0]["data"]["body"]["items"][0]["mentionCount"],
        1
    );
    assert_eq!(
        trace.events[0]["data"]["body"]["items"][0]["urgentCount"],
        1
    );
    assert_eq!(
        trace.events[0]["data"]["body"]["items"][0]["lastReadSeq"],
        38
    );
    assert_eq!(
        trace.events[0]["data"]["body"]["items"][0]["projectionRevision"],
        9
    );
    assert_eq!(trace.events[1]["event"], "im:channel:update");
    assert_eq!(trace.events[1]["data"]["channelId"], CHANNEL_ID);
    assert_eq!(trace.events[1]["data"]["unreadCount"], 0);
    assert_eq!(trace.events[1]["data"]["mentionCount"], 0);
    assert_eq!(trace.events[1]["data"]["mentionCountRoot"], 0);
    assert_eq!(trace.events[1]["data"]["lastReadSeq"], 41);
    assert_eq!(trace.events[1]["data"]["lastPostAt"], CREATED_AT - 10);
    assert_eq!(
        trace.events[1]["data"]["projectionAuthority"],
        "server-member-absolute"
    );
    assert_eq!(trace.events[2]["event"], "im:read:channel");
    assert_eq!(trace.events[2]["data"]["state"], "read");
    assert_eq!(trace.events[2]["data"]["unreadCount"], 0);
    assert_eq!(trace.events[2]["data"]["lastReadSeq"], 41);
    assert_eq!(trace.events[3]["event"], "im:read:result");
    assert_eq!(
        trace.events[3]["data"]["body"]["items"][0]["unreadCount"],
        0
    );
    assert_eq!(
        trace.events[3]["data"]["body"]["items"][0]["lastReadSeq"],
        41
    );
    Ok(())
}

/// Persist 失败时保留旧未读投影，不发布猜测出的 read 终态。
#[test]
fn g13_persist_failure_preserves_old_projection() -> TestResult<()> {
    let mut replay = Replay::new();
    let query_corr = replay.command_corr("im_query_dialog_list", json!({}))?;
    replay.reply(
        query_corr,
        serde_json::to_vec(&vec![dialog_row(3, 1, 1)]).map_err(|error| error.to_string())?,
        CREATED_AT as u64 + 1,
    )?;
    replay.reply_dialog_members(3, 1, 1, CREATED_AT as u64 + 1)?;
    let initial = replay.events()?;
    assert_eq!(initial.len(), 1);
    assert_eq!(initial[0]["event"], "im:read:result");
    assert_eq!(initial[0]["data"]["body"]["items"][0]["unreadCount"], 3);

    let view_corr = replay.command_corr(
        "im_channels_view",
        json!({"channels":[{"id":CHANNEL_ID,"isRoot":true}]}),
    )?;
    replay.reply(view_corr, http_success()?, CREATED_AT as u64 + 2)?;
    assert!(replay.events()?.is_empty());

    let persist_corr = replay.ingest_read()?;
    replay.reply_error(persist_corr, PortError::Storage(500), CREATED_AT as u64 + 3)?;
    let failed = replay.events()?;
    assert!(
        failed.is_empty(),
        "failure must preserve old projection and emit no guessed terminal: {failed:?}"
    );
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
    let mut trace = run_success()?;
    for ack in [false, true] {
        let reaction = run_reaction_success(ack)?;
        trace.events.extend(reaction.events);
        trace.ticks.extend(reaction.ticks);
    }
    let Some(output) = std::env::var_os("HELIX_GATE_ARTIFACT_DIR") else {
        return Ok(());
    };
    let bundle = artifact::gate_bundle(
        "G-13",
        "message_v3_g13",
        trace.ticks,
        trace.events,
        Vec::new(),
        json!([{"table":"channel_member","columns":["channel_id","user_id","unread_count","mention_count","urgent_mention_count","last_read_seq","reaction_unread_count"]}]),
        json!([{"table":"channel_member","columns":["channel_id","user_id","unread_count","mention_count","urgent_mention_count","last_read_seq","reaction_unread_count"]}]),
        json!([{"table":"channel_member","columns":["channel_id","user_id","unread_count","mention_count","urgent_mention_count","last_read_seq","reaction_unread_count"]}]),
    );
    artifact::write_bundle(std::path::Path::new(&output), &bundle)
        .map_err(|error| error.to_string())
        .map(|_| ())
}

/// 回归：个人未读回应必须持久化后输出，并拒绝同版本不同计数的读回。
#[test]
fn reaction_unread_member_projection_requires_matching_durable_count() -> TestResult<()> {
    for stored_count in [3, 4] {
        let mut replay = Replay::new();
        let query_corr = replay.command_corr("im_query_dialog_list", json!({}))?;
        replay.reply(
            query_corr,
            serde_json::to_vec(&vec![dialog_row(7, 0, 0)]).map_err(|error| error.to_string())?,
            CREATED_AT as u64,
        )?;
        replay.reply_dialog_members(7, 0, 0, CREATED_AT as u64)?;
        let wire = json!({"action":"member_projection","data":{
            "channel_id":CHANNEL_ID,"user_id":ACTOR_ID,
            "projection_revision":19,"effect_id":"reaction-effect-19",
            "unread_count":7,"last_read_seq":41,"reaction_unread_count":3
        }});
        replay.step(
            Tick::Inbound(InboundBytes(Bytes::from(wire.to_string()))),
            CREATED_AT as u64,
        )?;
        let persist_corr = replay.only_corr()?;
        assert!(replay.events()?.is_empty());
        let operations = format!("{:?}", replay.sink.as_slice());
        assert!(
            operations.contains("reaction_unread_count"),
            "reaction count was not persisted: {operations}"
        );
        assert!(
            !operations.contains("channel_event_cursor"),
            "private projection changed shared cursor"
        );
        replay.reply(persist_corr, Vec::new(), CREATED_AT as u64 + 1)?;
        assert!(replay.events()?.is_empty());
        let read_corr = replay.only_corr()?;
        replay.reply(
            read_corr,
            serde_json::to_vec(&json!([{
                "channel_id":CHANNEL_ID,"user_id":ACTOR_ID,
                "projection_revision":19,"effect_id":"reaction-effect-19",
                "unread_count":7,"last_read_seq":41,"reaction_unread_count":stored_count
            }]))
            .map_err(|error| error.to_string())?,
            CREATED_AT as u64 + 2,
        )?;
        let events = replay.events()?;
        if stored_count == 3 {
            assert!(
                events
                    .iter()
                    .any(|event| event["event"] == "im:channel:update"
                        && event["data"]["reactionUnreadCount"] == 3),
                "{events:?}"
            );
        } else {
            assert!(
                events.is_empty(),
                "conflicting durable count emitted success: {events:?}"
            );
        }
    }
    Ok(())
}

/// Construct the Go HTTP authority fixture, including a revision beyond JS integer precision.
fn reaction_response(count: i64) -> Value {
    json!({"status":"SUCCESS","data":{
        "count":count,
        "nextTarget":if count > 0 {json!({"postId":"reaction-post","revision":"9007199254740993"})} else {Value::Null},
        "memberProjection":{
            "channelId":CHANNEL_ID,"userId":ACTOR_ID,
            "projectionRevision":19,"effectId":"reaction-effect-19",
            "unreadCount":7,"lastReadSeq":41,"reactionUnreadCount":count
        }
    }})
}

/// Authorize the visible channel through the public dialog read before requesting personal state.
fn reaction_request(replay: &mut Replay, ack: bool) -> TestResult<Correlation> {
    let query = replay.command_corr("im_query_dialog_list", json!({}))?;
    replay.reply(
        query,
        serde_json::to_vec(&vec![dialog_row(7, 0, 0)]).map_err(|e| e.to_string())?,
        CREATED_AT as u64,
    )?;
    replay.reply_dialog_members(7, 0, 0, CREATED_AT as u64)?;
    replay.command_corr(if ack {"im_reaction_unread_ack"} else {"im_reaction_unread"},
        if ack {json!({"channel_id":CHANNEL_ID,"req_id":"reaction-request","post_id":"reaction-post","observed_revision":"9007199254740993"})}
        else {json!({"channel_id":CHANNEL_ID,"req_id":"reaction-request"})})
}

/// Use the real driver HTTP envelope at the public PortReply boundary.
fn reaction_http(body: &Value) -> TestResult<Vec<u8>> {
    Ok(json!({"status":200,"headers":{},"body":base64_std(&serde_json::to_vec(body).map_err(|e|e.to_string())?)}).to_string().into_bytes())
}

/// Query and ack return only after durable matching readback and finish exactly once.
#[test]
fn reaction_commands_wait_for_matching_persistence_and_ignore_duplicate_replies() -> TestResult<()>
{
    for ack in [false, true] {
        let mut replay = Replay::new();
        let http = reaction_request(&mut replay, ack)?;
        let body = reaction_response(if ack { 0 } else { 1 });
        replay.reply(http, reaction_http(&body)?, CREATED_AT as u64 + 1)?;
        assert!(replay.events()?.is_empty());
        let persist = replay.only_corr()?;
        assert!(!format!("{:?}", replay.sink.as_slice()).contains("channel_event_cursor"));
        replay.reply(persist, Vec::new(), CREATED_AT as u64 + 2)?;
        assert!(replay.events()?.is_empty());
        let readback = replay.only_corr()?;
        let row = json!([body["data"]["memberProjection"]]);
        replay.reply(
            readback,
            row.to_string().into_bytes(),
            CREATED_AT as u64 + 3,
        )?;
        let events = replay.events()?;
        let result = events
            .iter()
            .find(|e| e["event"] == "im:read:result")
            .ok_or("missing reaction terminal")?;
        assert_eq!(result["data"]["req_id"], "reaction-request");
        assert_eq!(result["data"]["body"], body);
        assert!(events.iter().any(|e| e["event"] == "im:channel:update"
            && e["data"]["reactionUnreadCount"] == body["data"]["count"]));
        for corr in [http, persist, readback] {
            replay.reply(corr, reaction_http(&body)?, CREATED_AT as u64 + 4)?;
            assert!(replay.events()?.is_empty());
            assert!(replay.sink.as_slice().is_empty());
        }
    }
    Ok(())
}

/// Every failed async stage emits one correlated error and rejects late success.
#[test]
fn reaction_commands_fail_closed_at_every_async_stage() -> TestResult<()> {
    for stage in 0..3 {
        for error in [
            PortError::Timeout,
            PortError::Network,
            PortError::Http(503),
            PortError::Storage(500),
        ] {
            let mut replay = Replay::new();
            let mut corr = reaction_request(&mut replay, true)?;
            let body = reaction_response(0);
            if stage >= 1 {
                replay.reply(corr, reaction_http(&body)?, CREATED_AT as u64 + 1)?;
                corr = replay.only_corr()?;
            }
            if stage >= 2 {
                replay.reply(corr, Vec::new(), CREATED_AT as u64 + 2)?;
                corr = replay.only_corr()?;
            }
            replay.reply_error(corr, error, CREATED_AT as u64 + 3)?;
            let events = replay.events()?;
            assert_eq!(events.len(), 1);
            assert_eq!(events[0]["event"], "im:read:result");
            assert!(events[0]["data"]["error"].is_string(), "{events:?}");
            replay.reply(corr, reaction_http(&body)?, CREATED_AT as u64 + 4)?;
            assert!(replay.sink.as_slice().is_empty());
        }
    }
    Ok(())
}

/// Stop cancels HTTP, persist and readback correlations; late replies cannot publish.
#[test]
fn reaction_commands_cancel_all_stages() -> TestResult<()> {
    for stage in 0..3 {
        let mut replay = Replay::new();
        let mut corr = reaction_request(&mut replay, false)?;
        let body = reaction_response(1);
        if stage >= 1 {
            replay.reply(corr, reaction_http(&body)?, CREATED_AT as u64 + 1)?;
            corr = replay.only_corr()?;
        }
        if stage >= 2 {
            replay.reply(corr, Vec::new(), CREATED_AT as u64 + 2)?;
            corr = replay.only_corr()?;
        }
        replay.sink.clear();
        replay
            .module
            .on_stop(&mut replay.sink)
            .map_err(|e| e.to_string())?;
        let events = replay.events()?;
        assert_eq!(
            events
                .iter()
                .filter(|e| e["event"] == "im:read:result"
                    && e["data"]["req_id"] == "reaction-request"
                    && e["data"]["error"] == "CANCELLED")
                .count(),
            1
        );
        replay.reply(corr, reaction_http(&body)?, CREATED_AT as u64 + 4)?;
        assert!(replay.sink.as_slice().is_empty());
    }
    Ok(())
}

/// Business errors, malformed counts/versions and another viewer's state never reach storage.
#[test]
fn reaction_commands_reject_untrusted_http_authority() -> TestResult<()> {
    let paths = [
        ("/status", json!("FAILED")),
        ("/data/count", json!(-1)),
        ("/data/nextTarget", Value::Null),
        ("/data/nextTarget/revision", json!(9007199254740993_u64)),
        ("/data/nextTarget/revision", json!("9223372036854775808")),
        ("/data/memberProjection/userId", json!("another-user")),
        ("/data/memberProjection/channelId", json!("another-channel")),
        ("/data/memberProjection/reactionUnreadCount", json!(2)),
        ("/data/memberProjection/effectId", json!("")),
    ];
    for (path, value) in paths {
        let mut replay = Replay::new();
        let corr = reaction_request(&mut replay, false)?;
        let mut body = reaction_response(1);
        *body.pointer_mut(path).ok_or("missing fixture pointer")? = value;
        replay.reply(corr, reaction_http(&body)?, CREATED_AT as u64 + 1)?;
        assert!(
            replay.only_corr().is_err(),
            "{path}: bad authority persisted"
        );
        let events = replay.events()?;
        assert_eq!(events.len(), 1);
        assert!(events[0]["data"]["error"].is_string(), "{path}: {events:?}");
    }
    Ok(())
}

/// A newer private WS projection may win before HTTP readback; never emit the stale count.
#[test]
fn reaction_commands_reject_superseded_durable_projection() -> TestResult<()> {
    let mut replay = Replay::new();
    let corr = reaction_request(&mut replay, true)?;
    let body = reaction_response(0);
    replay.reply(corr, reaction_http(&body)?, CREATED_AT as u64 + 1)?;
    let persist = replay.only_corr()?;
    replay.reply(persist, Vec::new(), CREATED_AT as u64 + 2)?;
    let readback = replay.only_corr()?;
    let mut row = body["data"]["memberProjection"].clone();
    row["projectionRevision"] = json!(20);
    row["effectId"] = json!("new-reaction");
    row["reactionUnreadCount"] = json!(1);
    replay.reply(
        readback,
        json!([row]).to_string().into_bytes(),
        CREATED_AT as u64 + 3,
    )?;
    let events = replay.events()?;
    assert_eq!(events.len(), 1);
    assert_eq!(
        events[0]["data"]["error"],
        "REACTION_UNREAD_PROJECTION_CHANGED"
    );
    Ok(())
}

/// Input validation protects the string revision and session-derived identity contract.
#[test]
fn reaction_commands_reject_invalid_input_without_http() -> TestResult<()> {
    for patch in [
        json!({"observed_revision":1}),
        json!({"observed_revision":"0"}),
        json!({"observed_revision":"-1"}),
        json!({"observed_revision":"1.2"}),
        json!({"observed_revision":"9223372036854775808"}),
        json!({"req_id":""}),
        json!({"post_id":""}),
        json!({"user_id":"forged"}),
        json!({"channel_id":""}),
    ] {
        let mut replay = Replay::new();
        let mut args = json!({"channel_id":CHANNEL_ID,"req_id":"reaction-request","post_id":"reaction-post","observed_revision":"9007199254740993"});
        args.as_object_mut()
            .ok_or("args object")?
            .extend(patch.as_object().ok_or("patch object")?.clone());
        let result = replay.step(
            Tick::Command(AppCommand::new(
                "im_reaction_unread_ack",
                args.to_string().into_bytes(),
            )),
            CREATED_AT as u64,
        );
        assert!(result.is_err(), "invalid request accepted: {args}");
        assert!(replay.sink.as_slice().is_empty());
    }
    Ok(())
}

/// Produce reaction canonical events from the public command and all durable replies.
fn run_reaction_success(ack: bool) -> TestResult<PersonaTrace> {
    let mut replay = Replay::new();
    let http = reaction_request(&mut replay, ack)?;
    let body = reaction_response(if ack { 0 } else { 1 });
    replay.reply(http, reaction_http(&body)?, CREATED_AT as u64 + 1)?;
    let persist = replay.only_corr()?;
    replay.reply(persist, Vec::new(), CREATED_AT as u64 + 2)?;
    let readback = replay.only_corr()?;
    replay.reply(
        readback,
        json!([body["data"]["memberProjection"]])
            .to_string()
            .into_bytes(),
        CREATED_AT as u64 + 3,
    )?;
    Ok(PersonaTrace {
        events: replay.events()?,
        ticks: vec![
            json!({"kind":"Command","value":{"name":if ack {"im_reaction_unread_ack"} else {"im_reaction_unread"},"payload":if ack {json!({"channel_id":CHANNEL_ID,"req_id":"reaction-request","post_id":"reaction-post","observed_revision":"9007199254740993"})} else {json!({"channel_id":CHANNEL_ID,"req_id":"reaction-request"})}},"now_ms":CREATED_AT}),
            json!({"kind":"PortReply","value":{"correlation":http.raw(),"outcome":{"variant":"Ok"}},"now_ms":CREATED_AT + 1}),
            json!({"kind":"PortReply","value":{"correlation":persist.raw(),"outcome":{"variant":"Ok"}},"now_ms":CREATED_AT + 2}),
            json!({"kind":"PortReply","value":{"correlation":readback.raw(),"outcome":{"variant":"Ok"}},"now_ms":CREATED_AT + 3}),
        ],
    })
}

/// Visible-message queries preserve empty filters and return exact string revisions after persistence.
#[test]
fn reaction_visible_query_roundtrips_bounded_targets() -> TestResult<()> {
    for ids in [
        json!([]),
        json!(["reaction-post", "other-post"]),
        json!(std::iter::once("reaction-post".to_string())
            .chain((1..100).map(|i| format!("post-{i}")))
            .collect::<Vec<_>>()),
    ] {
        let mut replay = Replay::new();
        let query = replay.command_corr("im_query_dialog_list", json!({}))?;
        replay.reply(
            query,
            json!([dialog_row(7, 0, 0)]).to_string().into_bytes(),
            CREATED_AT as u64,
        )?;
        let http = replay.command_corr(
            "im_reaction_unread",
            json!({
                "channel_id":CHANNEL_ID,"req_id":"visible","post_ids":ids
            }),
        )?;
        let sent = replay
            .sink
            .as_slice()
            .iter()
            .find_map(|effect| match effect {
                Effect::Http { req, .. } => req
                    .body
                    .as_ref()
                    .and_then(|body| serde_json::from_slice::<Value>(body.as_ref()).ok()),
                _ => None,
            })
            .ok_or("missing HTTP body")?;
        assert_eq!(sent["postIds"], ids);
        let mut body = reaction_response(1);
        body["data"]["targets"] = if ids.as_array().is_some_and(|ids| ids.is_empty()) {
            json!([])
        } else {
            json!([{"postId":"reaction-post","revision":"9007199254740993"}])
        };
        replay.reply(http, reaction_http(&body)?, CREATED_AT as u64 + 1)?;
        assert!(replay.events()?.is_empty());
        let persist = replay.only_corr()?;
        replay.reply(persist, Vec::new(), CREATED_AT as u64 + 2)?;
        let readback = replay.only_corr()?;
        replay.reply(
            readback,
            json!([body["data"]["memberProjection"]])
                .to_string()
                .into_bytes(),
            CREATED_AT as u64 + 3,
        )?;
        assert!(replay
            .events()?
            .iter()
            .any(|event| event["event"] == "im:read:result" && event["data"]["body"] == body));
    }
    Ok(())
}

/// Malformed or oversized visible filters fail before network dispatch.
#[test]
fn reaction_visible_query_rejects_invalid_filters() -> TestResult<()> {
    for ids in [
        Value::Null,
        json!("post"),
        json!([1]),
        json!([""]),
        json!([" "]),
        json!([" post"]),
        json!(["post", "post"]),
        json!((0..101).map(|i| format!("post-{i}")).collect::<Vec<_>>()),
    ] {
        let mut replay = Replay::new();
        let args = json!({"channel_id":CHANNEL_ID,"req_id":"visible","post_ids":ids});
        assert!(replay
            .step(
                Tick::Command(AppCommand::new(
                    "im_reaction_unread",
                    args.to_string().into_bytes()
                )),
                CREATED_AT as u64
            )
            .is_err());
        assert!(replay.sink.as_slice().is_empty());
    }
    Ok(())
}

/// New response targets obey the same revision boundary and cannot inflate the authority count.
#[test]
fn reaction_visible_query_rejects_invalid_targets() -> TestResult<()> {
    for targets in [
        Value::Null,
        json!({}),
        json!([{"postId":"","revision":"1"}]),
        json!([{"postId":"post","revision":1}]),
        json!([{"postId":"post","revision":"0"}]),
        json!([{"postId":"post","revision":"1"},{"postId":"post","revision":"2"}]),
        json!((0..101)
            .map(|i| json!({"postId":format!("post-{i}"),"revision":"1"}))
            .collect::<Vec<_>>()),
    ] {
        let mut replay = Replay::new();
        let corr = reaction_request(&mut replay, false)?;
        let mut body = reaction_response(1);
        body["data"]["targets"] = targets;
        replay.reply(corr, reaction_http(&body)?, CREATED_AT as u64 + 1)?;
        assert!(
            replay.only_corr().is_err(),
            "invalid targets reached storage: {body}"
        );
        assert!(replay
            .events()?
            .iter()
            .any(|event| event["data"]["error"].is_string()));
    }
    Ok(())
}
