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

pub const CHANNEL_ID: &str = "chfixx00000000000000000016";
pub const ACTOR_ID: &str = "g16-actor";
pub const TEAM_ID: &str = "g16-company";
pub const GROUP_NAME: &str = "G16 Fixture Group";

/// 保存 G16 初态与持久屏障后的唯一终态 channel-summary Frame。
pub struct G16LifecycleTrace {
    pub initial: InteractionFrameV2,
    pub terminal: InteractionFrameV2,
    pub persist_corr: Correlation,
    pub late_frame_count: usize,
}

/// 通过真实 `ImModule::handle` 驱动 close/leave authority 与持久屏障。
pub struct G16ScenarioRunner {
    harness: Harness,
}

impl G16ScenarioRunner {
    /// 建立固定身份、公司与确定性 correlation 空间。
    pub fn new() -> Self {
        Self {
            harness: Harness::new_with_identity(&[], ACTOR_ID, TEAM_ID, "", "", ""),
        }
    }

    /// 回放 owner close WS authority，PersistAtomic 成功后才释放 terminal Frame。
    pub fn run_closed() -> G16LifecycleTrace {
        let mut runner = Self::new();
        let initial = runner.attach_summary(false, false);
        runner.harness.step(
            helix_core::Tick::Inbound(InboundBytes(Bytes::from(
                serde_json::to_vec(&json!({
                    "action": "channel_close",
                    "data": {"channelId": CHANNEL_ID, "deleteAt": 1_700_001_600_010_i64}
                }))
                .expect("static G16 close authority serializes"),
            ))),
            1_700_001_600_010,
        );
        let persist_corr = runner.only_persist_corr(true);
        assert!(
            runner.frames().is_empty(),
            "close cannot publish before PersistOk"
        );
        runner
            .harness
            .step(port_reply_ok(persist_corr.raw()), 1_700_001_600_011);
        let terminal = runner.only_summary_frame();
        runner
            .harness
            .step(port_reply_ok(persist_corr.raw()), 1_700_001_600_012);
        G16LifecycleTrace {
            initial,
            terminal,
            persist_corr,
            late_frame_count: runner.frames().len(),
        }
    }

    /// 回放 self-leave HTTP SUCCESS 与后续本地 PersistOk。
    pub fn run_left() -> G16LifecycleTrace {
        let mut runner = Self::new();
        let initial = runner.attach_summary(false, false);
        runner.harness.step(
            command_json("im_channel_leave", json!({"channel_id": CHANNEL_ID})),
            1_700_001_600_020,
        );
        let http_corr = runner.only_http_corr();
        let body = serde_json::to_vec(&json!({"status":"SUCCESS","data":{}}))
            .expect("static G16 leave success serializes");
        runner.harness.step(
            port_reply_with_body(http_corr.raw(), enveloped(Bytes::from(body))),
            1_700_001_600_021,
        );
        let persist_corr = runner.only_persist_corr(false);
        assert!(
            runner.frames().is_empty(),
            "leave cannot publish before PersistOk"
        );
        runner
            .harness
            .step(port_reply_ok(persist_corr.raw()), 1_700_001_600_022);
        let terminal = runner.only_summary_frame();
        runner
            .harness
            .step(port_reply_ok(persist_corr.raw()), 1_700_001_600_023);
        G16LifecycleTrace {
            initial,
            terminal,
            persist_corr,
            late_frame_count: runner.frames().len(),
        }
    }

    /// 回放 HTTP business/transport failure 或 local persist failure 并返回终态 Frame 数。
    pub fn run_failure(leave_transport: bool, persist_failure: bool) -> usize {
        let mut runner = Self::new();
        runner.attach_summary(false, false);
        if leave_transport || !persist_failure {
            runner.harness.step(
                command_json("im_channel_leave", json!({"channel_id": CHANNEL_ID})),
                1_700_001_600_030,
            );
            let http_corr = runner.only_http_corr();
            if leave_transport {
                runner.harness.step(
                    helix_core::Tick::PortReply {
                        corr: http_corr,
                        outcome: PortOutcome::Err(PortError::Http(503)),
                    },
                    1_700_001_600_031,
                );
                return runner.frames().len();
            }
            let body = serde_json::to_vec(&json!({"status":"FAILED"}))
                .expect("static G16 leave rejection serializes");
            runner.harness.step(
                port_reply_with_body(http_corr.raw(), enveloped(Bytes::from(body))),
                1_700_001_600_031,
            );
            return runner.frames().len();
        }
        runner.harness.step(
            helix_core::Tick::Inbound(InboundBytes(Bytes::from(
                serde_json::to_vec(&json!({
                    "action":"channel_close",
                    "data":{"channelId":CHANNEL_ID,"deleteAt":1_700_001_600_032_i64}
                }))
                .expect("static G16 close authority serializes"),
            ))),
            1_700_001_600_032,
        );
        let persist_corr = runner.only_persist_corr(true);
        runner.harness.step(
            helix_core::Tick::PortReply {
                corr: persist_corr,
                outcome: PortOutcome::Err(PortError::Storage(16)),
            },
            1_700_001_600_033,
        );
        runner.frames().len()
    }

    /// 从 durable lifecycle row 重建 cold-start typed summary。
    pub fn run_recovery(left: bool) -> InteractionFrameV2 {
        let mut runner = Self::new();
        runner.attach_summary(!left, left)
    }

    /// attach channel-summary 并用 durable row 完成 snapshot readback。
    fn attach_summary(&mut self, closed: bool, left: bool) -> InteractionFrameV2 {
        self.harness.step(
            command_json(
                "__helix_request_surface_snapshot",
                json!({
                    "surface":"message.channel-summary",
                    "scope":{"kind":"channel","channelId":CHANNEL_ID}
                }),
            ),
            1_700_001_600_000,
        );
        let corr = self.only_persist_corr(false);
        self.harness.step(
            port_reply_with_body(corr.raw(), rows_bytes(json!([channel_row(closed, left)]))),
            1_700_001_600_001,
        );
        self.only_summary_frame()
    }

    /// 提取当前批次唯一 HTTP correlation。
    fn only_http_corr(&self) -> Correlation {
        only_corr(self.harness.sink.as_slice(), |effect| {
            matches!(effect, Effect::Http { .. })
        })
    }

    /// 提取当前批次唯一 Persist 或 PersistAtomic correlation。
    fn only_persist_corr(&self, atomic: bool) -> Correlation {
        only_corr(self.harness.sink.as_slice(), |effect| match effect {
            Effect::PersistAtomic { .. } => atomic,
            Effect::Persist { ops, .. } => {
                !atomic
                    && ops.iter().any(|op| {
                        matches!(
                            op,
                            StorageOp::Get(_)
                                | StorageOp::BatchUpdate(_)
                                | StorageOp::BatchUpsert(_)
                        )
                    })
            }
            _ => false,
        })
    }

    /// 解码当前 Effect 批次中的完整 V2 Frames。
    fn frames(&self) -> Vec<InteractionFrameV2> {
        decode_frames(self.harness.sink.as_slice())
    }

    /// 从可能同时存在的 raw domain emit 中提取唯一 channel-summary Frame。
    fn only_summary_frame(&self) -> InteractionFrameV2 {
        let mut frames = self.frames().into_iter().filter(|frame| {
            frame
                .mutations
                .iter()
                .any(|mutation| mutation.mutation.surface == SurfaceId::MessageChannelSummary)
        });
        let frame = frames.next().expect("one G16 channel-summary Frame");
        assert!(frames.next().is_none());
        frame
    }
}

/// 逐字段断言 G16 lifecycle Frame 的 envelope、scope、revision 与 terminal action。
pub fn assert_lifecycle_contract(frame: &InteractionFrameV2, expected_state: &str) {
    frame.validate().expect("G16 lifecycle Frame validates");
    assert_eq!(frame.schema_version, 2);
    assert_eq!(frame.mutations.len(), 1);
    let mutation = &frame.mutations[0];
    assert_eq!(mutation.mutation.surface, SurfaceId::MessageChannelSummary);
    assert_eq!(
        mutation.mutation.scope,
        SurfaceScope::Channel {
            channel_id: CHANNEL_ID.to_string(),
        }
    );
    let view = match &mutation.mutation.mutation {
        TypedSurfaceMutation::ChannelSummarySnapshot { view } => view,
        TypedSurfaceMutation::ChannelSummaryPatch { patch } => &patch.view,
        other => panic!("expected G16 channel-summary mutation, got {other:?}"),
    };
    assert_eq!(view.channel_id, CHANNEL_ID);
    assert_eq!(view.lifecycle.state, expected_state);
    if expected_state == "active" {
        assert!(view.lifecycle.reason.is_none());
    } else {
        assert!(view.lifecycle.reason.is_some());
        assert!(view.actions.iter().all(|action| !action.enabled));
    }
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
    let corr = correlations.next().expect("matching G16 correlation");
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
        .map(|bytes| InteractionFrameV2::decode(&bytes).expect("producer emits valid G16 V2"))
        .collect()
}

/// 构造 active、closed 或 left 的 durable channel read model。
fn channel_row(closed: bool, left: bool) -> Value {
    json!({
        "id": CHANNEL_ID,
        "display_name": GROUP_NAME,
        "role": if left {"MEMBER"} else {"OWNER"},
        "member_count": 2,
        "updated_at": 16,
        "delete_at": if closed {1_700_001_600_010_i64} else {0},
        "is_active": if closed {0} else {1},
        "is_remove": if left {1} else {0}
    })
}

/// 编码 storage get/scan 使用的 JSON row array。
fn rows_bytes(value: Value) -> Bytes {
    Bytes::from(serde_json::to_vec(&value).expect("static G16 storage rows serialize"))
}
