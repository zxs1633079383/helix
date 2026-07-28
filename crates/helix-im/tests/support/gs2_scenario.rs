use bytes::Bytes;
use helix_core::effect::{Effect, HttpRequest, StorageOp};
use helix_im::interaction_frame::{
    DialogListVm, InteractionFrameV2, PreviewVm, SurfaceScope, TypedSurfaceMutation,
};
use serde_json::{json, Value};

#[path = "../common.rs"]
#[allow(dead_code)]
mod common;
#[path = "../g01_shared/scenario.rs"]
#[allow(dead_code)]
mod shared;

use common::{
    cid, command_json, enveloped, ev, port_reply_err, port_reply_ok, port_reply_with_body,
    sync_reply_events_with_messages, Harness,
};
pub use shared::{assert_group_created_contract_for, SendScenarioSpec};

pub const CHANNEL_NUM: u64 = 22;
pub const CHANNEL_ID: &str = "chfixx00000000000000000016";
pub const ACTOR_ID: &str = "viewer-444";
pub const OBSERVER_ID: &str = "gs2-wdio-observer";
pub const TEAM_ID: &str = "gs2-wdio-company";
pub const GROUP_NAME: &str = "GS2 Offline Notify Group";
pub const MESSAGE_ID: &str = "gs2-offline-message";

/// 返回 GS2 fresh-group、sync 与 Fixture 共享的稳定身份。
pub fn gs2_scenario_spec() -> SendScenarioSpec {
    SendScenarioSpec {
        channel_id: CHANNEL_ID,
        server_id: "srvfix00000000000000000022",
        request_id: "gs2-offline-notify-request",
        marker: "gs2-offline-notify",
        simple_message: "gs2-offline-notify",
        message_type: "TEXT",
        props: json!({}),
        actor_id: ACTOR_ID,
        observer_id: OBSERVER_ID,
        team_id: TEAM_ID,
        created_at: 1_700_002_220_000,
        group_name: GROUP_NAME,
        api_base_url: "http://gs2-wdio.invalid",
    }
}

/// 汇总 GS2 HTTP、原子持久、最终 Frame 与表序。
pub struct Gs2SuccessTrace {
    pub group_created: InteractionFrameV2,
    pub initial: InteractionFrameV2,
    pub terminal: InteractionFrameV2,
    pub request: HttpRequest,
    pub persist_tables: Vec<&'static str>,
}

/// 汇总 GS2 fail-closed 分支的可观察数量。
pub struct Gs2NoFrameTrace {
    pub frame_count: usize,
    pub refresh_count: usize,
}

/// 通过生产 ImModule command/PortReply 路径执行 GS2。
pub struct Gs2ScenarioRunner;

impl Gs2ScenarioRunner {
    /// 回放 fresh group、explicit sync、PersistOk、durable Get 与最终 Frame。
    pub fn run_success() -> Gs2SuccessTrace {
        let group_created = group_created_frame();
        let mut replay = Replay::new();
        let initial = replay.attach_dialog(baseline_channel_rows());
        replay.attach_timeline();
        let (sync_corr, request) = replay.start_sync();
        replay.reply(
            port_reply_with_body(sync_corr, sync_response()),
            1_700_002_220_020,
        );
        assert!(replay.frames().is_empty());
        let (persist_corr, persist_tables) = replay.only_atomic_persist();
        replay.reply(port_reply_ok(persist_corr), 1_700_002_220_021);
        assert!(replay.frames().is_empty());
        let refresh_corr = replay.only_channel_get_corr();
        replay.reply(
            port_reply_with_body(refresh_corr, channel_rows()),
            1_700_002_220_022,
        );
        Gs2SuccessTrace {
            group_created,
            initial,
            terminal: replay.only_frame(),
            request,
            persist_tables,
        }
    }

    /// 回放 HTTP 失败，证明 transport failure 不触发持久或 Frame。
    pub fn run_http_failure() -> Gs2NoFrameTrace {
        let mut replay = Replay::new();
        replay.attach_dialog(baseline_channel_rows());
        replay.attach_timeline();
        let (sync_corr, _) = replay.start_sync();
        replay.reply(port_reply_err(sync_corr), 1_700_002_220_020);
        replay.no_frame_trace()
    }

    /// 回放 PersistAtomic 失败，证明 authority facts 不形成可见终态。
    pub fn run_persist_failure() -> Gs2NoFrameTrace {
        let mut replay = Replay::new();
        replay.attach_dialog(baseline_channel_rows());
        replay.attach_timeline();
        let (sync_corr, _) = replay.start_sync();
        replay.reply(
            port_reply_with_body(sync_corr, sync_response()),
            1_700_002_220_020,
        );
        let (persist_corr, _) = replay.only_atomic_persist();
        replay.reply(port_reply_err(persist_corr), 1_700_002_220_021);
        replay.no_frame_trace()
    }

    /// 回放已消费 PersistOk 的迟到重复回包，证明不会重复读表或出帧。
    pub fn run_late_persist_reply() -> Gs2NoFrameTrace {
        let mut replay = Replay::new();
        replay.attach_dialog(baseline_channel_rows());
        replay.attach_timeline();
        let (sync_corr, _) = replay.start_sync();
        replay.reply(
            port_reply_with_body(sync_corr, sync_response()),
            1_700_002_220_020,
        );
        let (persist_corr, _) = replay.only_atomic_persist();
        replay.reply(port_reply_ok(persist_corr), 1_700_002_220_021);
        let refresh_corr = replay.only_channel_get_corr();
        replay.reply(
            port_reply_with_body(refresh_corr, channel_rows()),
            1_700_002_220_022,
        );
        replay.only_frame();
        replay.reply(port_reply_ok(persist_corr), 1_700_002_220_023);
        replay.no_frame_trace()
    }

    /// 仅从 durable channel row attach，证明恢复不依赖 raw sync event。
    pub fn run_recovery() -> InteractionFrameV2 {
        let mut replay = Replay::new();
        replay.attach_dialog(channel_rows())
    }
}

/// 验证 GS2 terminal Frame 只包含 Helix 决定的 dialog typed state。
pub fn assert_terminal_contract(frame: &InteractionFrameV2) {
    frame.validate().expect("GS2 terminal Frame remains valid");
    assert_eq!(frame.schema_version, 2);
    let scope = SurfaceScope::User {
        user_id: ACTOR_ID.to_string(),
        team_id: TEAM_ID.to_string(),
    };
    let dialog = frame
        .mutations
        .iter()
        .find_map(|mutation| {
            assert_eq!(mutation.mutation.scope, scope);
            match &mutation.mutation.mutation {
                TypedSurfaceMutation::DialogListSnapshot { view } => Some(view),
                TypedSurfaceMutation::DialogListPatch { patch } => Some(&patch.view),
                _ => None,
            }
        })
        .expect("GS2 publishes dialog state");
    assert_dialog_view(dialog);
}

/// 验证离线消息已进入 dialog preview/unread 的完整 typed row。
fn assert_dialog_view(view: &DialogListVm) {
    let row = view
        .sections
        .iter()
        .flat_map(|section| section.items.iter())
        .find(|row| row.channel_id == CHANNEL_ID)
        .unwrap_or_else(|| panic!("GS2 channel is visible, sections={:?}", view.sections));
    assert_eq!(row.title.text, GROUP_NAME);
    assert_eq!(row.unread_count, 1);
    assert!(matches!(
        &row.preview,
        PreviewVm::Text { text, .. } if text == "GS2 offline message"
    ));
}

/// 用共享生产建群回放生成 GS2 fresh-group 前置 Frame。
fn group_created_frame() -> InteractionFrameV2 {
    let mut runner = shared::G01ScenarioRunner::new_for(gs2_scenario_spec());
    runner.create_group_prerequisite()
}

/// 封装 GS2 production Harness 与当前 step Effects。
struct Replay {
    harness: Harness,
}

impl Replay {
    /// 创建带稳定 actor/team 和初始 cursor=0 的 GS2 Harness。
    fn new() -> Self {
        Self {
            harness: Harness::new_with_identity(
                &[CHANNEL_NUM],
                ACTOR_ID,
                TEAM_ID,
                "GS2 Actor",
                "",
                "",
            ),
        }
    }

    /// attach dialog 并消费初始 channel Scan。
    fn attach_dialog(&mut self, rows: Bytes) -> InteractionFrameV2 {
        self.harness
            .step(command_json("im_query_dialog_list", json!({})), 1);
        let corr = self
            .harness
            .sink
            .as_slice()
            .iter()
            .find_map(|effect| match effect {
                Effect::Persist { corr, ops }
                    if ops.iter().any(
                        |op| matches!(op, StorageOp::Scan(scan) if scan.table == "channel"),
                    ) =>
                {
                    Some(corr.raw())
                }
                _ => None,
            })
            .expect("dialog attach scans channel");
        self.reply(port_reply_with_body(corr, rows), 2);
        self.only_frame()
    }

    /// attach latest timeline，使 recovery batch 仅刷新当前可见频道。
    fn attach_timeline(&mut self) {
        self.harness.step(
            command_json(
                "im_query_messages_by_channel",
                json!({
                    "channel_id": CHANNEL_ID,
                    "limit": 20,
                    "windowToken": "latest"
                }),
            ),
            3,
        );
        let corr = self
            .harness
            .sink
            .as_slice()
            .iter()
            .find_map(|effect| match effect {
                Effect::Persist { corr, ops }
                    if ops.iter().any(
                        |op| matches!(op, StorageOp::Scan(scan) if scan.table == "message"),
                    ) =>
                {
                    Some(corr.raw())
                }
                _ => None,
            })
            .expect("timeline attach scans message");
        self.reply(port_reply_with_body(corr, Bytes::from_static(b"[]")), 4);
        let remote_corr = self
            .harness
            .sink
            .as_slice()
            .iter()
            .find_map(|effect| match effect {
                Effect::Http { corr, req } if req.url.ends_with("/posts/getLatestPost") => {
                    Some(corr.raw())
                }
                _ => None,
            })
            .expect("empty local timeline falls back to latest-post HTTP");
        self.reply(
            port_reply_with_body(
                remote_corr,
                enveloped(Bytes::from_static(br#"{"status":"SUCCESS","data":[]}"#)),
            ),
            5,
        );
        self.only_frame();
        self.harness.step(
            command_json(
                "__helix_request_surface_snapshot",
                json!({
                    "surface": "message.timeline",
                    "scope": {
                        "kind": "timeline",
                        "channelId": CHANNEL_ID,
                        "windowToken": "latest"
                    },
                    "causationId": "gs2-flow-attach"
                }),
            ),
            6,
        );
        assert!(
            self.harness.sink.as_slice().iter().any(|effect| matches!(
                effect,
                Effect::Persist { ops, .. }
                    if ops.iter().any(
                        |op| matches!(op, StorageOp::Scan(scan) if scan.table == "message"),
                    )
            )),
            "surface snapshot registers the attached timeline before its readback"
        );
    }

    /// 提交 explicit sync 并返回唯一 sync/notify correlation 与请求。
    fn start_sync(&mut self) -> (u64, HttpRequest) {
        self.harness.step(
            command_json(
                "im_sync_channels",
                json!({
                    "channel_id": CHANNEL_ID,
                    "reason": "offline-recovery",
                    "event_seq": 999_999_u64
                }),
            ),
            1_700_002_220_010,
        );
        self.harness
            .sink
            .as_slice()
            .iter()
            .find_map(|effect| match effect {
                Effect::Http { corr, req } if req.url.ends_with("/channel/sync/notify") => {
                    Some((corr.raw(), req.clone()))
                }
                _ => None,
            })
            .expect("explicit sync emits one sync/notify request")
    }

    /// 返回唯一 PersistAtomic correlation 与有序表名。
    fn only_atomic_persist(&self) -> (u64, Vec<&'static str>) {
        self.harness
            .sink
            .as_slice()
            .iter()
            .find_map(|effect| match effect {
                Effect::PersistAtomic { corr, ops } => {
                    Some((corr.raw(), ops.iter().filter_map(table_name).collect()))
                }
                _ => None,
            })
            .expect("sync authority emits PersistAtomic")
    }

    /// 返回 PersistOk 后唯一 channel Get correlation。
    fn only_channel_get_corr(&self) -> u64 {
        self.harness
            .sink
            .as_slice()
            .iter()
            .find_map(|effect| match effect {
                Effect::Persist { corr, ops }
                    if ops
                        .iter()
                        .any(|op| matches!(op, StorageOp::Get(get) if get.table == "channel")) =>
                {
                    Some(corr.raw())
                }
                _ => None,
            })
            .expect("PersistOk schedules durable channel Get")
    }

    /// 回灌一个 port reply 并推进生产 module。
    fn reply(&mut self, tick: helix_core::Tick, now_ms: u64) {
        self.harness.step(tick, now_ms);
    }

    /// 解码当前 step 唯一 InteractionFrameV2。
    fn only_frame(&self) -> InteractionFrameV2 {
        let frames = self.frames();
        assert_eq!(frames.len(), 1, "expected exactly one GS2 Frame");
        frames.into_iter().next().expect("one GS2 Frame exists")
    }

    /// 收集当前 step 的完整 InteractionFrameV2。
    fn frames(&self) -> Vec<InteractionFrameV2> {
        self.harness
            .sink
            .as_slice()
            .iter()
            .filter_map(|effect| match effect {
                Effect::Emit { event } => {
                    let envelope: Value = serde_json::from_slice(event.0.as_ref()).ok()?;
                    (envelope["event"] == "im:interaction:frame").then(|| {
                        InteractionFrameV2::decode(
                            &serde_json::to_vec(&envelope["data"])
                                .expect("GS2 Frame data serializes"),
                        )
                        .expect("GS2 Frame decodes")
                    })
                }
                _ => None,
            })
            .collect()
    }

    /// 汇总当前 step 的 Frame 与 channel refresh 数量。
    fn no_frame_trace(&self) -> Gs2NoFrameTrace {
        let refresh_count = self
            .harness
            .sink
            .as_slice()
            .iter()
            .filter(|effect| {
                matches!(
                    effect,
                    Effect::Persist { ops, .. }
                        if ops.iter().any(
                            |op| matches!(op, StorageOp::Get(get) if get.table == "channel"),
                        )
                )
            })
            .count();
        Gs2NoFrameTrace {
            frame_count: self.frames().len(),
            refresh_count,
        }
    }
}

/// 返回 StorageOp 的稳定表名用于事务顺序断言。
fn table_name(op: &StorageOp) -> Option<&'static str> {
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

/// 构造一条可见离线消息的真 Go sync envelope。
fn sync_response() -> Bytes {
    enveloped(sync_reply_events_with_messages(
        CHANNEL_NUM,
        &[ev(1, 1, MESSAGE_ID)],
        &[(MESSAGE_ID, "GS2 offline message", "")],
        1,
    ))
}

/// 构造 PersistOk 后从本地 channel authority 回读的 dialog row。
fn channel_rows() -> Bytes {
    Bytes::from(
        serde_json::to_vec(&json!([{
            "id": cid(CHANNEL_NUM),
            "display_name": GROUP_NAME,
            "team_id": TEAM_ID,
            "type": "P",
            "member_count": 2,
            "unread_count": 1,
            "mention_count": 0,
            "urgent_count": 0,
            "mention_list": [],
            "urgent_post_list": [],
            "unread_post_id": MESSAGE_ID,
            "last_post_at": 1_700_002_220_000_i64,
            "last_post": json!({
                "id": MESSAGE_ID,
                "channelId": CHANNEL_ID,
                "userId": OBSERVER_ID,
                "message":"GS2 offline message",
                "type": "TEXT",
                "createAt": 1_700_002_220_000_i64
            }).to_string(),
            "picture_type":"USER",
            "picture":json!({"userIds":[ACTOR_ID,OBSERVER_ID]}).to_string()
        }]))
        .expect("GS2 channel rows serialize"),
    )
}

/// 构造 sync 前已持久的 fresh-group baseline dialog row。
fn baseline_channel_rows() -> Bytes {
    Bytes::from(
        serde_json::to_vec(&json!([{
            "id": cid(CHANNEL_NUM),
            "display_name": GROUP_NAME,
            "type":"P",
            "team_id":TEAM_ID,
            "member_count":2,
            "unread_count":0,
            "mention_count":0,
            "urgent_count":0,
            "last_post_at":1_700_002_219_000_i64,
            "last_post":json!({
                "id":"gs2-baseline-message",
                "channelId":CHANNEL_ID,
                "userId":ACTOR_ID,
                "message":"GS2 baseline",
                "type":"TEXT",
                "createAt":1_700_002_219_000_i64
            }).to_string(),
            "picture_type":"USER",
            "picture":json!({"userIds":[ACTOR_ID,OBSERVER_ID]}).to_string()
        }]))
        .expect("GS2 baseline rows serialize"),
    )
}
