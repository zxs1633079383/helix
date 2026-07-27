use bytes::Bytes;
use helix_core::effect::{Effect, HttpRequest, StorageOp};
use helix_core::tick::{AppCommand, InboundBytes, PortError, PortOutcome, ReplyBytes};
use helix_core::{Correlation, EffectSink, Module, Tick};
use helix_im::interaction_frame::{
    DialogListVm, InteractionFrameV2, PreviewVm, SurfaceScope, TypedSurfaceMutation,
};
use helix_im::module::ImConfig;
use helix_im::ImModule;
use serde_json::{json, Value};

#[path = "../g01_shared/scenario.rs"]
#[allow(dead_code)]
mod shared;

#[allow(unused_imports)]
pub use shared::{assert_group_created_contract_for, SendScenarioSpec};

pub const CHANNEL_ID: &str = "chfixx00000000000000000021";
pub const ACTOR_ID: &str = "gs1-wdio-actor";
pub const OBSERVER_ID: &str = "gs1-wdio-observer";
pub const TEAM_ID: &str = "gs1-wdio-company";
pub const GROUP_NAME: &str = "GS1 Cold Start Group";
pub const CREATED_AT: i64 = 1_700_001_710_000;

/// 返回 GS1 fresh-group 前置与冷启动同步共用的确定性身份。
pub fn gs1_scenario_spec() -> SendScenarioSpec {
    SendScenarioSpec {
        channel_id: CHANNEL_ID,
        server_id: "srvfix00000000000000000021",
        request_id: "gs1-cold-start-request",
        marker: "gs1-cold-start",
        simple_message: "gs1-cold-start",
        message_type: "TEXT",
        props: json!({}),
        actor_id: ACTOR_ID,
        observer_id: OBSERVER_ID,
        team_id: TEAM_ID,
        created_at: CREATED_AT,
        group_name: GROUP_NAME,
        api_base_url: "http://gs1-wdio.invalid",
    }
}

/// 汇总 hello、批次持久屏障与最终 dialog Frame 的生产 trace。
pub struct Gs1SuccessTrace {
    pub group_created: InteractionFrameV2,
    pub initial: InteractionFrameV2,
    pub terminal: InteractionFrameV2,
    pub increment_request: HttpRequest,
    pub persist_corr: Correlation,
    pub persist_tables: Vec<&'static str>,
}

/// 汇总 GS1 fail-closed 分支可观察结果。
pub struct Gs1NoFrameTrace {
    pub frame_count: usize,
    pub refresh_scan_count: usize,
}

/// 只通过公开 `ImModule::handle` 驱动 GS1 hello/increment/end/PortReply。
pub struct Gs1ScenarioRunner;

impl Gs1ScenarioRunner {
    /// 回放 fresh group、hello、完整增量批次、PersistOk 与 durable dialog scan。
    pub fn run_success() -> Gs1SuccessTrace {
        let group_created = group_created_frame();
        let mut replay = Replay::new();
        let initial = replay.attach_dialog(json!([]));
        replay.hello();
        let increment_request = replay.only_http_request("/channels/load/increment");
        replay.ingest_increment(valid_increment_data());
        let (persist_corr, persist_tables) = replay.finish_increment_batch();
        assert!(
            replay.frames().is_empty(),
            "global end cannot publish GS1 Frame before PersistOk"
        );
        replay.reply(
            persist_corr,
            PortOutcome::Ok(ReplyBytes(Bytes::new())),
            CREATED_AT as u64 + 30,
        );
        let refresh_corr = replay.only_channel_scan_corr();
        assert!(
            replay.frames().is_empty(),
            "PersistOk only schedules the durable dialog scan"
        );
        replay.reply(
            refresh_corr,
            PortOutcome::Ok(ReplyBytes(rows_bytes(json!([durable_dialog_row()])))),
            CREATED_AT as u64 + 31,
        );
        Gs1SuccessTrace {
            group_created,
            initial,
            terminal: replay.only_frame(),
            increment_request,
            persist_corr,
            persist_tables,
        }
    }

    /// 回放批次持久失败，证明不会扫描或发布最终 dialog Frame。
    pub fn run_persist_failure() -> Gs1NoFrameTrace {
        let mut replay = Replay::new();
        replay.attach_dialog(json!([]));
        replay.hello();
        replay.ingest_increment(valid_increment_data());
        let (persist_corr, _) = replay.finish_increment_batch();
        replay.reply(
            persist_corr,
            PortOutcome::Err(PortError::Storage(500)),
            CREATED_AT as u64 + 30,
        );
        Gs1NoFrameTrace {
            frame_count: replay.frames().len(),
            refresh_scan_count: replay.channel_scan_count(),
        }
    }

    /// 回放非法业务帧，证明 global end 不凭空形成持久事务或 Frame。
    pub fn run_invalid_increment() -> Gs1NoFrameTrace {
        let mut replay = Replay::new();
        replay.attach_dialog(json!([]));
        replay.hello();
        replay.ingest_increment(json!({"displayName":"missing channel id"}));
        replay.step(increment_end_tick(), CREATED_AT as u64 + 20);
        Gs1NoFrameTrace {
            frame_count: replay.frames().len(),
            refresh_scan_count: replay.channel_scan_count(),
        }
    }

    /// 回放已消费 correlation 的迟到 PersistOk，证明不会重复扫描或出帧。
    pub fn run_late_persist_reply() -> Gs1NoFrameTrace {
        let mut replay = Replay::new();
        replay.attach_dialog(json!([]));
        replay.hello();
        replay.ingest_increment(valid_increment_data());
        let (persist_corr, _) = replay.finish_increment_batch();
        replay.reply(
            persist_corr,
            PortOutcome::Ok(ReplyBytes(Bytes::new())),
            CREATED_AT as u64 + 30,
        );
        let refresh_corr = replay.only_channel_scan_corr();
        replay.reply(
            refresh_corr,
            PortOutcome::Ok(ReplyBytes(rows_bytes(json!([durable_dialog_row()])))),
            CREATED_AT as u64 + 31,
        );
        replay.only_frame();
        replay.reply(
            persist_corr,
            PortOutcome::Ok(ReplyBytes(Bytes::new())),
            CREATED_AT as u64 + 32,
        );
        Gs1NoFrameTrace {
            frame_count: replay.frames().len(),
            refresh_scan_count: replay.channel_scan_count(),
        }
    }

    /// 从 durable channel row 冷 attach，证明恢复不依赖平台缓存或 raw increment。
    pub fn run_recovery() -> InteractionFrameV2 {
        let mut replay = Replay::new();
        replay.attach_dialog(json!([durable_dialog_row()]))
    }
}

/// 逐字段验证 GS1 冷启动最终 dialog typed view。
pub fn assert_terminal_contract(frame: &InteractionFrameV2) {
    frame.validate().expect("GS1 terminal Frame remains valid");
    assert_eq!(frame.schema_version, 2);
    assert!(frame.directives.is_empty());
    let expected_scope = SurfaceScope::User {
        user_id: ACTOR_ID.to_string(),
        team_id: TEAM_ID.to_string(),
    };
    let dialog = frame
        .mutations
        .iter()
        .find_map(|mutation| {
            assert_eq!(mutation.mutation.scope, expected_scope);
            match &mutation.mutation.mutation {
                TypedSurfaceMutation::DialogListSnapshot { view } => Some(view),
                _ => None,
            }
        })
        .expect("GS1 publishes a dialog snapshot");
    assert_dialog_view(dialog);
}

/// 验证最终窗口只含服务端持久事实，不从 raw event 补字段。
fn assert_dialog_view(view: &DialogListVm) {
    let row = view
        .sections
        .iter()
        .flat_map(|section| section.items.iter())
        .find(|row| row.channel_id == CHANNEL_ID)
        .expect("GS1 durable channel row is visible");
    assert_eq!(row.title.text, GROUP_NAME);
    assert_eq!(row.member_count, 2);
    assert_eq!(row.unread_count, 1);
    assert!(matches!(
        &row.preview,
        PreviewVm::Text { text, .. } if text == "GS1 offline hello"
    ));
}

/// 用共享生产建群回放生成 GS1 fresh-group prerequisite Frame。
fn group_created_frame() -> InteractionFrameV2 {
    let mut runner = shared::G01ScenarioRunner::new_for(gs1_scenario_spec());
    runner.create_group_prerequisite()
}

/// 封装公开生产 module 与当前 step Effects。
struct Replay {
    module: ImModule,
    sink: EffectSink,
}

impl Replay {
    /// 创建带固定 actor/team/api 的 GS1 production module。
    fn new() -> Self {
        Self {
            module: ImModule::new(ImConfig {
                api_base_url: "http://gs1-wdio.invalid".to_string(),
                default_api_base_url: "http://gs1-wdio.invalid".to_string(),
                auth_user_id: ACTOR_ID.to_string(),
                company_id: TEAM_ID.to_string(),
                user_name: "GS1 Fixture Actor".to_string(),
                ..ImConfig::default()
            }),
            sink: EffectSink::new(),
        }
    }

    /// 执行一个生产 Tick，并把 sink 重置为该 step 的 Effects。
    fn step(&mut self, tick: Tick, now_ms: u64) {
        self.sink.clear();
        self.module
            .handle(&tick, now_ms, &mut self.sink)
            .expect("GS1 production step succeeds");
    }

    /// 通过 canonical dialog query attach renderer surface。
    fn attach_dialog(&mut self, rows: Value) -> InteractionFrameV2 {
        self.step(
            command("im_query_dialog_list", json!({"limit":500})),
            CREATED_AT as u64,
        );
        let corr = self.only_channel_scan_corr();
        self.reply(
            corr,
            PortOutcome::Ok(ReplyBytes(rows_bytes(rows))),
            CREATED_AT as u64 + 1,
        );
        self.only_frame()
    }

    /// 注入服务端 hello 并断言单次全量 increment HTTP。
    fn hello(&mut self) {
        self.step(hello_tick(), CREATED_AT as u64 + 10);
        assert_eq!(
            self.sink
                .as_slice()
                .iter()
                .filter(|effect| matches!(effect, Effect::Http { req, .. } if req.url.ends_with("/channels/load/increment")))
                .count(),
            1,
            "one hello schedules exactly one bootstrap increment HTTP"
        );
    }

    /// 注入一条生产 registry `increment_channel` 帧。
    fn ingest_increment(&mut self, data: Value) {
        self.step(increment_tick(data), CREATED_AT as u64 + 20);
    }

    /// 注入 global end 并返回唯一批次 Persist 与表序。
    fn finish_increment_batch(&mut self) -> (Correlation, Vec<&'static str>) {
        self.step(increment_end_tick(), CREATED_AT as u64 + 21);
        let matches = self
            .sink
            .as_slice()
            .iter()
            .filter_map(|effect| match effect {
                Effect::Persist { corr, ops } => Some((*corr, op_tables(ops))),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(matches.len(), 1, "GS1 global end creates one batch Persist");
        matches.into_iter().next().expect("one batch Persist exists")
    }

    /// 回灌一个相关端口结果。
    fn reply(&mut self, corr: Correlation, outcome: PortOutcome, now_ms: u64) {
        self.step(Tick::PortReply { corr, outcome }, now_ms);
    }

    /// 返回当前 step 唯一 channel Scan correlation。
    fn only_channel_scan_corr(&self) -> Correlation {
        let correlations = self
            .sink
            .as_slice()
            .iter()
            .filter_map(|effect| match effect {
                Effect::Persist { corr, ops }
                    if ops.iter().any(
                        |op| matches!(op, StorageOp::Scan(scan) if scan.table == "channel"),
                    ) =>
                {
                    Some(*corr)
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(correlations.len(), 1, "expected one channel Scan");
        correlations[0]
    }

    /// 返回当前 step 的 channel Scan 数。
    fn channel_scan_count(&self) -> usize {
        self.sink
            .as_slice()
            .iter()
            .filter(|effect| matches!(
                effect,
                Effect::Persist { ops, .. }
                    if ops.iter().any(
                        |op| matches!(op, StorageOp::Scan(scan) if scan.table == "channel"),
                    )
            ))
            .count()
    }

    /// 返回当前 step 唯一匹配 URL 的完整 HTTP request。
    fn only_http_request(&self, suffix: &str) -> HttpRequest {
        let requests = self
            .sink
            .as_slice()
            .iter()
            .filter_map(|effect| match effect {
                Effect::Http { req, .. } if req.url.ends_with(suffix) => Some(req.clone()),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(requests.len(), 1, "expected one GS1 HTTP request");
        requests[0].clone()
    }

    /// 解码当前 step 的全部 InteractionFrameV2 Emit。
    fn frames(&self) -> Vec<InteractionFrameV2> {
        self.sink
            .as_slice()
            .iter()
            .filter_map(|effect| match effect {
                Effect::Emit { event } => decode_frame(event.0.as_ref()),
                _ => None,
            })
            .collect()
    }

    /// 返回当前 step 唯一 InteractionFrameV2。
    fn only_frame(&self) -> InteractionFrameV2 {
        let frames = self.frames();
        assert_eq!(frames.len(), 1, "expected one GS1 Frame");
        frames.into_iter().next().expect("one GS1 Frame exists")
    }
}

/// 解码 im:interaction:frame 外层事件中的 V2 payload。
fn decode_frame(bytes: &[u8]) -> Option<InteractionFrameV2> {
    let envelope: Value = serde_json::from_slice(bytes).ok()?;
    (envelope.get("event")?.as_str()? == "im:interaction:frame")
        .then(|| serde_json::from_value(envelope.get("data")?.clone()).ok())
        .flatten()
}

/// 构造公开 command ingress。
fn command(name: &'static str, payload: Value) -> Tick {
    Tick::Command(AppCommand::new(
        name,
        serde_json::to_vec(&payload).expect("GS1 command serializes"),
    ))
}

/// 构造服务端 hello ingress。
fn hello_tick() -> Tick {
    inbound_tick(json!({
        "v":1,
        "action":"hello",
        "data":{"connectionId":"CONN_GS1"}
    }))
}

/// 构造生产 `increment_channel` ingress。
fn increment_tick(data: Value) -> Tick {
    inbound_tick(json!({"v":1,"action":"increment_channel","data":data}))
}

/// 构造 global `increment_channel_end` ingress。
fn increment_end_tick() -> Tick {
    inbound_tick(json!({"v":1,"action":"increment_channel_end","data":{}}))
}

/// 把静态 JSON 包装成 driver InboundBytes。
fn inbound_tick(value: Value) -> Tick {
    Tick::Inbound(InboundBytes(Bytes::from(
        serde_json::to_vec(&value).expect("GS1 inbound serializes"),
    )))
}

/// 返回 parser/channel writer 所需的最小合法 increment authority。
fn valid_increment_data() -> Value {
    json!({
        "id":CHANNEL_ID,
        "displayName":GROUP_NAME,
        "type":"P",
        "teamId":TEAM_ID,
        "createAt":CREATED_AT,
        "createBy":ACTOR_ID,
        "lastEventSeq":0,
        "needSync":false,
        "memberCount":2,
        "memberChange":{"join":[
            {"id":ACTOR_ID,"teamId":TEAM_ID,"role":"CREATOR","nickName":""},
            {"id":OBSERVER_ID,"teamId":TEAM_ID,"role":"MEMBER","nickName":""}
        ]},
        "owner":{"id":ACTOR_ID,"teamId":TEAM_ID},
        "pictureType":"USER",
        "picture":{"userIds":[ACTOR_ID,OBSERVER_ID]},
        "unreadCount":1,
        "mentionCount":0,
        "urgentCount":0,
        "mentionList":[],
        "urgentPostList":[]
    })
}

/// 返回持久成功后 channel Scan 的 render-ready authority row。
fn durable_dialog_row() -> Value {
    json!({
        "id":CHANNEL_ID,
        "display_name":GROUP_NAME,
        "type":"P",
        "team_id":TEAM_ID,
        "member_count":2,
        "unread_count":1,
        "mention_count":0,
        "urgent_count":0,
        "last_post_at":CREATED_AT - 10,
        "last_post":json!({
            "id":"srvfix00000000000000000021",
            "channelId":CHANNEL_ID,
            "userId":OBSERVER_ID,
            "message":"GS1 offline hello",
            "type":"TEXT",
            "createAt":CREATED_AT - 10
        }).to_string(),
        "picture_type":"USER",
        "picture":json!({"userIds":[ACTOR_ID,OBSERVER_ID]}).to_string()
    })
}

/// 返回批次持久操作的表顺序。
fn op_tables(ops: &[StorageOp]) -> Vec<&'static str> {
    ops.iter().filter_map(op_table).collect()
}

/// 提取单个 StorageOp 的表名。
fn op_table(op: &StorageOp) -> Option<&'static str> {
    match op {
        StorageOp::BatchUpsert(spec) => Some(spec.table),
        StorageOp::BatchUpdate(spec) => Some(spec.table),
        StorageOp::BatchDelete(spec) => Some(spec.table),
        StorageOp::MonotonicUpsert(spec) => Some(spec.table),
        StorageOp::GuardedBump(spec) => Some(spec.table),
        StorageOp::Get(spec) => Some(spec.table),
        StorageOp::Scan(spec) => Some(spec.table),
    }
}

/// 序列化 storage reply rows。
fn rows_bytes(rows: Value) -> Bytes {
    Bytes::from(serde_json::to_vec(&rows).expect("GS1 rows serialize"))
}
