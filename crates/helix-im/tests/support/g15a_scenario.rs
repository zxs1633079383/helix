use bytes::Bytes;
use helix_core::effect::{Effect, StorageOp};
use helix_core::tick::{InboundBytes, PortError, PortOutcome};
use helix_core::Correlation;
use helix_im::interaction_frame::{
    InteractionFrameV2, SurfaceId, SurfaceScope, TypedSurfaceMutation,
};
use serde_json::{json, Value};

#[path = "../common.rs"]
mod common;
use common::{command_json, enveloped, port_reply_ok, port_reply_with_body, Harness};

pub const CHANNEL_ID: &str = "chfixx00000000000000000015";
pub const REQUEST_ID: &str = "g15a-create-request-1";
pub const ACTOR_ID: &str = "g15a-actor";
pub const OBSERVER_ID: &str = "g15a-observer";
pub const TEAM_ID: &str = "g15a-company";
pub const GROUP_NAME: &str = "G15a Fixture Group";

/// G-15a happy/race trace 只暴露 durable terminal Frame 和因果标识。
pub struct G15aCreatedTrace {
    pub created: InteractionFrameV2,
    pub persist_corr: Correlation,
}

/// G-15a recovery trace 分别保留 storage 重放的 summary 与 roster Frame。
pub struct G15aRecoveryTrace {
    pub summary: InteractionFrameV2,
    pub roster: InteractionFrameV2,
}

/// 通过生产 `ImModule::handle` 驱动 G-15a command/HTTP/WS/Persist/Surface 回放。
pub struct G15aScenarioRunner {
    harness: Harness,
}

impl G15aScenarioRunner {
    /// 建立固定身份和 API 边界的确定性 runner。
    pub fn new() -> Self {
        Self {
            harness: Harness::new_with_identity(
                &[],
                ACTOR_ID,
                TEAM_ID,
                "",
                "http://g15a.invalid",
                "http://g15a.invalid",
            ),
        }
    }

    /// 回放 HTTP authority、PersistAtomic、duplicate WS 与 late HTTP。
    pub fn run_http_created() -> G15aCreatedTrace {
        let mut runner = Self::new();
        let http_corr = runner.start_create();
        runner.reply_http_success(http_corr);
        let persist_corr = runner.only_persist_atomic_corr();
        assert!(
            runner.frames().is_empty(),
            "HTTP authority cannot publish before PersistAtomic"
        );
        runner
            .harness
            .step(port_reply_ok(persist_corr.raw()), 1_700_001_500_002);
        let created = runner.only_created_frame();
        runner
            .harness
            .step(channel_created_inbound(), 1_700_001_500_003);
        assert!(runner.harness.sink.as_slice().is_empty());
        runner
            .harness
            .step(port_reply_ok(http_corr.raw()), 1_700_001_500_004);
        assert!(runner.harness.sink.as_slice().is_empty());
        G15aCreatedTrace {
            created,
            persist_corr,
        }
    }

    /// 回放 HTTP transport/business failure，二者都不得产生持久化或 Frame。
    pub fn run_http_failure(business: bool) -> usize {
        let mut runner = Self::new();
        let http_corr = runner.start_create();
        if business {
            let body = serde_json::to_vec(&json!({
                "status": "FAILED",
                "message": "create rejected"
            }))
            .expect("static business failure serializes");
            runner.harness.step(
                port_reply_with_body(http_corr.raw(), enveloped(Bytes::from(body))),
                1_700_001_500_001,
            );
        } else {
            runner.harness.step(
                helix_core::Tick::PortReply {
                    corr: http_corr,
                    outcome: PortOutcome::Err(PortError::Http(503)),
                },
                1_700_001_500_001,
            );
        }
        runner.harness.sink.as_slice().len()
    }

    /// 回放持久失败后同一 WS authority 的安全重试。
    pub fn run_persist_failure_replay() -> InteractionFrameV2 {
        let mut runner = Self::new();
        runner
            .harness
            .step(channel_created_inbound(), 1_700_001_500_000);
        let failed_corr = runner.only_persist_atomic_corr();
        runner.harness.step(
            helix_core::Tick::PortReply {
                corr: failed_corr,
                outcome: PortOutcome::Err(PortError::Storage(1)),
            },
            1_700_001_500_001,
        );
        assert!(runner.frames().is_empty());
        runner
            .harness
            .step(channel_created_inbound(), 1_700_001_500_002);
        let replay_corr = runner.only_persist_atomic_corr();
        assert_ne!(failed_corr, replay_corr);
        runner
            .harness
            .step(port_reply_ok(replay_corr.raw()), 1_700_001_500_003);
        runner.only_created_frame()
    }

    /// 从 durable channel/channel_member rows 冷启动恢复两个 typed slots。
    pub fn run_recovery() -> G15aRecoveryTrace {
        let mut runner = Self::new();
        runner.attach_summary();
        let summary_corr = runner.only_persist_corr_for("channel");
        runner.harness.step(
            port_reply_with_body(summary_corr.raw(), rows_bytes(json!([channel_row()]))),
            1_700_001_500_010,
        );
        let summary = runner.only_frame();
        runner.attach_roster();
        let roster_corr = runner.only_persist_corr_for("channel_member");
        runner.harness.step(
            port_reply_with_body(roster_corr.raw(), rows_bytes(json!(member_rows()))),
            1_700_001_500_011,
        );
        let roster = runner.only_frame();
        G15aRecoveryTrace { summary, roster }
    }

    /// 提交生产 create command 并返回其唯一 HTTP correlation。
    fn start_create(&mut self) -> Correlation {
        self.harness.step(
            command_json(
                "im_create_channel",
                json!({
                    "type": "O",
                    "user_ids": [OBSERVER_ID],
                    "display_name": GROUP_NAME,
                    "force_create": true,
                    "req_id": REQUEST_ID
                }),
            ),
            1_700_001_500_000,
        );
        self.only_http_corr()
    }

    /// 回灌带 owner 的权威 SUCCESS channel 对象。
    fn reply_http_success(&mut self, corr: Correlation) {
        let body = serde_json::to_vec(&json!({
            "status": "SUCCESS",
            "data": channel_authority()
        }))
        .expect("static create success serializes");
        self.harness.step(
            port_reply_with_body(corr.raw(), enveloped(Bytes::from(body))),
            1_700_001_500_001,
        );
    }

    /// 请求 channel-summary storage recovery。
    fn attach_summary(&mut self) {
        self.harness.step(
            command_json(
                "__helix_request_surface_snapshot",
                json!({
                    "surface": "message.channel-summary",
                    "scope": {"kind": "channel", "channelId": CHANNEL_ID}
                }),
            ),
            1_700_001_500_009,
        );
    }

    /// 请求 created cursor 下的 member-roster storage recovery。
    fn attach_roster(&mut self) {
        self.harness.step(
            command_json(
                "__helix_request_surface_snapshot",
                json!({
                    "surface": "message.member-roster",
                    "scope": {
                        "kind": "memberRoster",
                        "channelId": CHANNEL_ID,
                        "cursor": "created"
                    }
                }),
            ),
            1_700_001_500_010,
        );
    }

    /// 提取当前批次唯一 HTTP correlation。
    fn only_http_corr(&self) -> Correlation {
        only_corr(self.harness.sink.as_slice(), |effect| {
            matches!(effect, Effect::Http { .. })
        })
    }

    /// 提取当前批次唯一 PersistAtomic correlation。
    fn only_persist_atomic_corr(&self) -> Correlation {
        only_corr(self.harness.sink.as_slice(), |effect| {
            matches!(effect, Effect::PersistAtomic { .. })
        })
    }

    /// 按读表提取 recovery Persist correlation。
    fn only_persist_corr_for(&self, table: &str) -> Correlation {
        only_corr(self.harness.sink.as_slice(), |effect| match effect {
            Effect::Persist { ops, .. } => ops.iter().any(|op| match op {
                StorageOp::Get(spec) => spec.table == table,
                StorageOp::Scan(spec) => spec.table == table,
                _ => false,
            }),
            _ => false,
        })
    }

    /// 解码当前批次全部 V2 Frame。
    fn frames(&self) -> Vec<InteractionFrameV2> {
        decode_frames(self.harness.sink.as_slice())
    }

    /// 提取当前批次唯一 V2 Frame。
    fn only_frame(&self) -> InteractionFrameV2 {
        let frames = self.frames();
        assert_eq!(frames.len(), 1);
        frames.into_iter().next().expect("one frame")
    }

    /// 从可能同时含 dialog 的提交批次提取 summary+roster Frame。
    fn only_created_frame(&self) -> InteractionFrameV2 {
        let mut matches = self.frames().into_iter().filter(|frame| {
            frame
                .mutations
                .iter()
                .any(|mutation| mutation.mutation.surface == SurfaceId::MessageChannelSummary)
        });
        let frame = matches.next().expect("created typed Frame");
        assert!(matches.next().is_none());
        frame
    }
}

/// 逐字段断言 G-15a terminal Frame 的 envelope、summary、roster 与 actions。
pub fn assert_created_contract(frame: &InteractionFrameV2, causation: Option<&str>) {
    frame.validate().expect("G15a Frame validates");
    assert_eq!(frame.schema_version, 2);
    assert_eq!(frame.causation_id.as_deref(), causation);
    assert!(frame.directives.is_empty());
    assert_eq!(frame.mutations.len(), 2);
    let summary = frame
        .mutations
        .iter()
        .find(|mutation| mutation.mutation.surface == SurfaceId::MessageChannelSummary)
        .expect("summary mutation");
    assert_eq!(
        summary.mutation.scope,
        SurfaceScope::Channel {
            channel_id: CHANNEL_ID.to_string()
        }
    );
    assert_eq!(summary.mutation.causation_id.as_deref(), causation);
    let summary_view = match &summary.mutation.mutation {
        TypedSurfaceMutation::ChannelSummarySnapshot { view } => view,
        other => panic!("expected summary snapshot, got {other:?}"),
    };
    assert_eq!(summary_view.channel_id, CHANNEL_ID);
    assert_eq!(summary_view.title.text, GROUP_NAME);
    assert_eq!(summary_view.member_count, 2);
    assert_eq!(format!("{:?}", summary_view.viewer_role), "Owner");
    assert!(summary_view
        .actions
        .iter()
        .any(|action| action.kind == "channel-settings" && action.enabled));
    assert!(summary_view
        .actions
        .iter()
        .any(|action| action.kind == "channel-close" && action.enabled));
    let roster = frame
        .mutations
        .iter()
        .find(|mutation| mutation.mutation.surface == SurfaceId::MessageMemberRoster)
        .expect("roster mutation");
    assert_eq!(
        roster.mutation.scope,
        SurfaceScope::MemberRoster {
            channel_id: CHANNEL_ID.to_string(),
            cursor: "first".to_string()
        }
    );
    assert_eq!(roster.mutation.causation_id.as_deref(), causation);
    let roster_view = match &roster.mutation.mutation {
        TypedSurfaceMutation::MemberRosterSnapshot { view } => view,
        other => panic!("expected roster snapshot, got {other:?}"),
    };
    assert_eq!(roster_view.channel_id, CHANNEL_ID);
    assert_eq!(roster_view.member_count, 2);
    assert_eq!(
        roster_view
            .items
            .iter()
            .map(|item| item.user.user_id.as_str())
            .collect::<Vec<_>>(),
        vec![OBSERVER_ID, ACTOR_ID]
    );
}

/// 从 Effect 批次提取唯一匹配 correlation。
fn only_corr(effects: &[Effect], predicate: impl Fn(&Effect) -> bool) -> Correlation {
    let mut correlations = effects
        .iter()
        .filter(|effect| predicate(effect))
        .filter_map(|effect| match effect {
            Effect::Http { corr, .. }
            | Effect::Persist { corr, .. }
            | Effect::PersistAtomic { corr, .. } => Some(*corr),
            _ => None,
        });
    let corr = correlations.next().expect("matching correlation");
    assert!(correlations.next().is_none());
    corr
}

/// 解码 Effect::Emit 中的完整 InteractionFrameV2。
fn decode_frames(effects: &[Effect]) -> Vec<InteractionFrameV2> {
    effects
        .iter()
        .filter_map(|effect| {
            let Effect::Emit { event } = effect else {
                return None;
            };
            let envelope: Value = serde_json::from_slice(event.0.as_ref()).ok()?;
            (envelope["event"] == "im:interaction:frame")
                .then(|| serde_json::to_vec(&envelope["data"]).ok())
                .flatten()
        })
        .map(|bytes| InteractionFrameV2::decode(&bytes).expect("producer emits valid V2"))
        .collect()
}

/// 构造 WS channel_created authority Tick。
fn channel_created_inbound() -> helix_core::Tick {
    let bytes = serde_json::to_vec(&json!({
        "action": "channel_created",
        "data": channel_authority()
    }))
    .expect("static inbound serializes");
    helix_core::Tick::Inbound(InboundBytes(Bytes::from(bytes)))
}

/// 返回 HTTP/WS 共用的新频道权威对象。
fn channel_authority() -> Value {
    json!({
        "id": CHANNEL_ID,
        "type": "P",
        "displayName": GROUP_NAME,
        "teamId": TEAM_ID,
        "createAt": 1_700_001_500_000_i64,
        "reqId": REQUEST_ID,
        "members": [
            {"id": OBSERVER_ID, "role": "MEMBER", "nickName": "Observer"}
        ],
        "owner": {"id": ACTOR_ID, "role": "OWNER", "nickName": "Actor"}
    })
}

/// 返回 storage channel row 的 snake_case 形态。
fn channel_row() -> Value {
    json!({
        "id": CHANNEL_ID,
        "display_name": GROUP_NAME,
        "member_count": 2,
        "role": "OWNER",
        "created_at": 1_700_001_500_000_i64
    })
}

/// 返回 durable roster rows。
fn member_rows() -> Vec<Value> {
    vec![
        json!({"channel_id": CHANNEL_ID, "user_id": OBSERVER_ID, "role": "MEMBER", "nick_name": "Observer"}),
        json!({"channel_id": CHANNEL_ID, "user_id": ACTOR_ID, "role": "OWNER", "nick_name": "Actor"}),
    ]
}

/// 编码 storage scan/get 的 JSON row 数组。
fn rows_bytes(value: Value) -> Bytes {
    Bytes::from(serde_json::to_vec(&value).expect("static storage rows serialize"))
}
