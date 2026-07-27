use bytes::Bytes;
use helix_core::effect::{Effect, StorageOp};
use helix_core::tick::{InboundBytes, PortError, PortOutcome};
use helix_core::Correlation;
use helix_im::interaction_frame::{
    DetailContextVm, InteractionFrameV2, SurfaceId, SurfaceScope, TypedSurfaceMutation,
};
use serde_json::{json, Value};

#[path = "../common.rs"]
mod common;
use common::{command_json, enveloped, port_reply_ok, port_reply_with_body, Harness};

pub const PARENT_CHANNEL_ID: &str = "chfixx00000000000000000014";
pub const TOPIC_CHANNEL_ID: &str = "chfixx00000000000000000015";
pub const ROOT_MESSAGE_ID: &str = "srvfix00000000000000000015";
pub const REQUEST_ID: &str = "g15-topic-request-1";
pub const ACTOR_ID: &str = "g15-actor";
pub const OBSERVER_ID: &str = "g15-observer";
pub const TEAM_ID: &str = "g15-company";
pub const TOPIC_TITLE: &str = "G15 Fixture Topic";

/// G-15 happy trace 保留 terminal typed Frame 及两个 durable barrier correlation。
#[allow(dead_code)]
pub struct G15CreatedTrace {
    pub thread: InteractionFrameV2,
    pub relation_persist_corr: Correlation,
    pub root_read_corr: Correlation,
}

/// G-15 production recovery 只从 durable topic channel row 重建 summary。
pub struct G15RecoveryTrace {
    pub summary: InteractionFrameV2,
}

/// G-15 WS authority 可能同批发布 dialog 与 topic summary/roster，调用方按 surface 取独立 Frame。
pub struct G15WsCreatedTrace {
    pub frames: Vec<InteractionFrameV2>,
}

/// 通过生产 `ImModule::handle` 驱动 make-topic 的 command/HTTP/Persist/readback 回放。
pub struct G15ScenarioRunner {
    harness: Harness,
}

impl G15ScenarioRunner {
    /// 建立固定身份与 API 边界的确定性 runner。
    pub fn new() -> Self {
        Self {
            harness: Harness::new_with_identity(&[], ACTOR_ID, TEAM_ID, "", "", ""),
        }
    }

    /// 回放 HTTP authority、relation PersistOk 与 durable root readback。
    pub fn run_http_created() -> G15CreatedTrace {
        let mut runner = Self::new();
        let http_corr = runner.start_topic();
        runner.reply_http_success(http_corr);
        let relation_persist_corr = runner.only_persist_corr_for_write();
        assert!(runner.frames().is_empty());
        runner.harness.step(
            port_reply_ok(relation_persist_corr.raw()),
            1_700_001_510_002,
        );
        let root_read_corr = runner.only_persist_corr_for_read("message");
        assert!(runner.frames().is_empty());
        runner.reply_root_read(root_read_corr);
        let thread = runner.only_frame();
        runner
            .harness
            .step(port_reply_ok(root_read_corr.raw()), 1_700_001_510_004);
        assert!(runner.harness.sink.as_slice().is_empty());
        G15CreatedTrace {
            thread,
            relation_persist_corr,
            root_read_corr,
        }
    }

    /// 回放 HTTP transport/business failure，二者都不得产生持久化或 Frame。
    #[allow(dead_code)]
    pub fn run_http_failure(business: bool) -> usize {
        let mut runner = Self::new();
        let http_corr = runner.start_topic();
        if business {
            let body = serde_json::to_vec(&json!({
                "status": "FAILED",
                "message": "topic rejected"
            }))
            .expect("static business failure serializes");
            runner.harness.step(
                port_reply_with_body(http_corr.raw(), enveloped(Bytes::from(body))),
                1_700_001_510_001,
            );
        } else {
            runner.harness.step(
                helix_core::Tick::PortReply {
                    corr: http_corr,
                    outcome: PortOutcome::Err(PortError::Http(503)),
                },
                1_700_001_510_001,
            );
        }
        runner.harness.sink.as_slice().len()
    }

    /// 回放 relation persist failure，证明 readback 与 terminal Frame 都不可达。
    #[allow(dead_code)]
    pub fn run_persist_failure() -> usize {
        let mut runner = Self::new();
        let http_corr = runner.start_topic();
        runner.reply_http_success(http_corr);
        let persist_corr = runner.only_persist_corr_for_write();
        runner.harness.step(
            helix_core::Tick::PortReply {
                corr: persist_corr,
                outcome: PortOutcome::Err(PortError::Storage(1)),
            },
            1_700_001_510_002,
        );
        runner.frames().len()
    }

    /// 回放 durable topic channel 的 cold summary recovery。
    pub fn run_recovery() -> G15RecoveryTrace {
        let mut runner = Self::new();
        runner.harness.step(
            command_json(
                "__helix_request_surface_snapshot",
                json!({
                    "surface": "message.channel-summary",
                    "scope": {"kind": "channel", "channelId": TOPIC_CHANNEL_ID}
                }),
            ),
            1_700_001_510_010,
        );
        let read_corr = runner.only_persist_corr_for_read("channel");
        runner.harness.step(
            port_reply_with_body(read_corr.raw(), rows_bytes(json!([topic_channel_row()]))),
            1_700_001_510_011,
        );
        G15RecoveryTrace {
            summary: runner.only_frame(),
        }
    }

    /// 回放独立 `channel_created` WS authority 及其 PersistAtomic terminal Frames。
    pub fn run_ws_created() -> G15WsCreatedTrace {
        let mut runner = Self::new();
        runner.harness.step(
            command_json(
                "__helix_request_surface_snapshot",
                json!({
                    "surface": "message.left-dialog-list",
                    "scope": {"kind": "user", "userId": ACTOR_ID, "teamId": TEAM_ID}
                }),
            ),
            1_700_001_510_018,
        );
        let dialog_read_corr = only_corr(runner.harness.sink.as_slice(), |effect| {
            matches!(effect, Effect::Persist { .. })
        });
        runner.harness.step(
            port_reply_with_body(dialog_read_corr.raw(), rows_bytes(json!([]))),
            1_700_001_510_019,
        );
        runner.harness.step(
            helix_core::Tick::Inbound(InboundBytes(Bytes::from(
                serde_json::to_vec(&json!({
                    "action": "channel_created",
                    "data": {
                        "id": TOPIC_CHANNEL_ID,
                        "type": "T",
                        "displayName": TOPIC_TITLE,
                        "teamId": TEAM_ID,
                        "rootId": ROOT_MESSAGE_ID,
                        "createAt": 1_700_001_510_000_i64,
                        "reqId": REQUEST_ID,
                        "members": [
                            {"id": OBSERVER_ID, "role": "MEMBER", "nickName": "Observer"}
                        ],
                        "owner": {"id": ACTOR_ID, "role": "OWNER", "nickName": "Actor"}
                    }
                }))
                .expect("static G15 channel_created serializes"),
            ))),
            1_700_001_510_020,
        );
        let persist_corr = only_corr(runner.harness.sink.as_slice(), |effect| {
            matches!(effect, Effect::PersistAtomic { .. })
        });
        runner
            .harness
            .step(port_reply_ok(persist_corr.raw()), 1_700_001_510_021);
        G15WsCreatedTrace {
            frames: runner.frames(),
        }
    }

    /// 提交 production make-topic command 并返回唯一 HTTP correlation。
    fn start_topic(&mut self) -> Correlation {
        self.harness.step(
            command_json(
                "im_make_topic",
                json!({
                    "root_id": ROOT_MESSAGE_ID,
                    "user_ids": [OBSERVER_ID],
                    "display_name": TOPIC_TITLE,
                    "req_id": REQUEST_ID
                }),
            ),
            1_700_001_510_000,
        );
        only_corr(self.harness.sink.as_slice(), |effect| {
            matches!(effect, Effect::Http { .. })
        })
    }

    /// 回灌 root/topic/revision 的权威 SUCCESS 响应。
    fn reply_http_success(&mut self, corr: Correlation) {
        let body = serde_json::to_vec(&json!({
            "status": "SUCCESS",
            "data": {
                "rootMessageId": ROOT_MESSAGE_ID,
                "channelId": TOPIC_CHANNEL_ID,
                "revision": 41
            }
        }))
        .expect("static topic authority serializes");
        self.harness.step(
            port_reply_with_body(corr.raw(), enveloped(Bytes::from(body))),
            1_700_001_510_001,
        );
    }

    /// 回灌 relation commit 后读取到的 root message durable row。
    fn reply_root_read(&mut self, corr: Correlation) {
        self.harness.step(
            port_reply_with_body(corr.raw(), rows_bytes(json!([root_message_row()]))),
            1_700_001_510_003,
        );
    }

    /// 提取当前批次唯一 relation write correlation。
    fn only_persist_corr_for_write(&self) -> Correlation {
        only_corr(self.harness.sink.as_slice(), |effect| match effect {
            Effect::Persist { ops, .. } => ops
                .iter()
                .any(|op| matches!(op, StorageOp::BatchUpsert(_) | StorageOp::BatchUpdate(_))),
            _ => false,
        })
    }

    /// 按表提取当前批次唯一 durable readback correlation。
    fn only_persist_corr_for_read(&self, table: &str) -> Correlation {
        only_corr(self.harness.sink.as_slice(), |effect| match effect {
            Effect::Persist { ops, .. } => ops
                .iter()
                .any(|op| matches!(op, StorageOp::Get(spec) if spec.table == table)),
            _ => false,
        })
    }

    /// 解码当前 Effect 批次内的全部完整 V2 Frame。
    fn frames(&self) -> Vec<InteractionFrameV2> {
        decode_frames(self.harness.sink.as_slice())
    }

    /// 提取当前批次唯一完整 V2 Frame。
    fn only_frame(&self) -> InteractionFrameV2 {
        let frames = self.frames();
        assert_eq!(frames.len(), 1);
        frames.into_iter().next().expect("one G15 Frame")
    }
}

/// 逐字段断言 G-15 terminal thread Frame 的 envelope、topic context 与 root。
pub fn assert_thread_contract(frame: &InteractionFrameV2) {
    frame.validate().expect("G15 thread Frame validates");
    assert_eq!(frame.schema_version, 2);
    assert_eq!(frame.causation_id.as_deref(), Some(REQUEST_ID));
    assert!(frame.directives.is_empty());
    assert_eq!(frame.mutations.len(), 1);
    let mutation = &frame.mutations[0];
    assert_eq!(mutation.mutation.surface, SurfaceId::MessageThreadDetail);
    assert_eq!(
        mutation.mutation.scope,
        SurfaceScope::Thread {
            channel_id: TOPIC_CHANNEL_ID.to_string(),
            root_message_id: ROOT_MESSAGE_ID.to_string(),
        }
    );
    let view = match &mutation.mutation.mutation {
        TypedSurfaceMutation::ThreadDetailSnapshot { view } => view,
        other => panic!("expected thread snapshot, got {other:?}"),
    };
    assert_eq!(view.channel_id, TOPIC_CHANNEL_ID);
    assert_eq!(view.root.id, ROOT_MESSAGE_ID);
    assert_eq!(view.root.channel_id, TOPIC_CHANNEL_ID);
    assert!(view.ordered_reply_ids.is_empty());
    assert!(view.nodes.is_empty());
    assert_eq!(view.reply_count, 0);
    assert_eq!(
        view.detail_context,
        DetailContextVm::Topic {
            parent_channel_id: PARENT_CHANNEL_ID.to_string(),
            topic_channel_id: TOPIC_CHANNEL_ID.to_string(),
            root_message_id: ROOT_MESSAGE_ID.to_string(),
            title: TOPIC_TITLE.to_string(),
        }
    );
}

/// 逐字段断言 G-15 cold recovery 的 channel-summary Frame。
pub fn assert_summary_contract(frame: &InteractionFrameV2) {
    frame.validate().expect("G15 summary Frame validates");
    assert_eq!(frame.schema_version, 2);
    assert_eq!(frame.causation_id, None);
    assert_eq!(frame.mutations.len(), 1);
    let mutation = &frame.mutations[0];
    assert_eq!(mutation.mutation.surface, SurfaceId::MessageChannelSummary);
    let view = match &mutation.mutation.mutation {
        TypedSurfaceMutation::ChannelSummarySnapshot { view } => view,
        other => panic!("expected summary snapshot, got {other:?}"),
    };
    assert_eq!(view.channel_id, TOPIC_CHANNEL_ID);
    assert_eq!(view.title.text, TOPIC_TITLE);
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
    let corr = correlations.next().expect("matching G15 correlation");
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
        .map(|bytes| InteractionFrameV2::decode(&bytes).expect("producer emits valid G15 V2"))
        .collect()
}

/// 返回 make-topic root 的 durable storage row。
fn root_message_row() -> Value {
    json!({
        "id": ROOT_MESSAGE_ID,
        "channel_id": PARENT_CHANNEL_ID,
        "user_id": ACTOR_ID,
        "create_at": 1_700_001_510_000_i64,
        "type": "TEXT",
        "message": "G15 root message",
        "props": {}
    })
}

/// 返回 topic channel 的 durable storage row。
fn topic_channel_row() -> Value {
    json!({
        "id": TOPIC_CHANNEL_ID,
        "type": "T",
        "root_post_id": ROOT_MESSAGE_ID,
        "display_name": TOPIC_TITLE,
        "member_count": 2,
        "role": "OWNER",
        "updated_at": 41
    })
}

/// 编码 storage get/scan 的 JSON row array。
fn rows_bytes(value: Value) -> Bytes {
    Bytes::from(serde_json::to_vec(&value).expect("static G15 storage rows serialize"))
}
