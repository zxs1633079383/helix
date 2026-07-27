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
pub const ACTOR_ID: &str = "g16c-actor";
pub const TEAM_ID: &str = "g16c-company";
pub const REQUEST_ID: &str = "g16c-settings-request";

/// 保存 G16c 初始快照、持久屏障与最终 typed settings patch。
pub struct G16cSettingsTrace {
    pub initial: InteractionFrameV2,
    pub terminal: InteractionFrameV2,
    pub persist_corr: Correlation,
}

/// 通过生产 `ImModule::handle` 驱动设置 HTTP/WS authority。
pub struct G16cScenarioRunner {
    harness: Harness,
}

impl G16cScenarioRunner {
    /// 建立固定 viewer identity 与确定性 correlation 空间。
    pub fn new() -> Self {
        Self {
            harness: Harness::new_with_identity(&[], ACTOR_ID, TEAM_ID, "", "", ""),
        }
    }

    /// 回放聚合 info 成功、PersistOk 与 typed channel-summary patch。
    pub fn run_info() -> G16cSettingsTrace {
        Self::run_http_command(
            "im_channel_change_info",
            json!({
                "channel_id":CHANNEL_ID,
                "purpose":"客户端简介",
                "header":"客户端规则",
                "expected_version":7,
                "req_id":REQUEST_ID
            }),
            json!({
                "channelId":CHANNEL_ID,
                "displayName":"G16c Renamed",
                "purpose":"服务端简介",
                "header":"服务端规则",
                "settingVersion":8,
                "capabilitiesByRole":{"CREATOR":{"canManageSettings":true}}
            }),
            8,
        )
    }

    /// 回放专用 displayName 命令并证明同一持久化 Frame 合同。
    pub fn run_display_name() -> G16cSettingsTrace {
        Self::run_http_command(
            "im_channel_change_display_name",
            json!({
                "channel_id":CHANNEL_ID,
                "display_name":"G16c Display",
                "req_id":"g16c-display-request"
            }),
            json!({
                "channelId":CHANNEL_ID,
                "displayName":"G16c Display",
                "purpose":"旧简介",
                "header":"旧规则",
                "settingVersion":9,
                "capabilitiesByRole":{"CREATOR":{"canManageSettings":true}}
            }),
            9,
        )
    }

    /// 回放专用 notice 命令并证明公告摘要进入同一 typed summary。
    pub fn run_notice() -> G16cSettingsTrace {
        Self::run_http_command(
            "im_channel_change_notice",
            json!({
                "channel_id":CHANNEL_ID,
                "notice":{"content":"G16c 公告"},
                "req_id":"g16c-notice-request"
            }),
            json!({
                "channelId":CHANNEL_ID,
                "displayName":"G16c Group",
                "purpose":"旧简介",
                "header":"G16c 公告",
                "settingVersion":10,
                "capabilitiesByRole":{"CREATOR":{"canManageSettings":true}}
            }),
            10,
        )
    }

    /// 回放 observer `update_channel`，只在 WS 字段持久成功后发布 patch。
    pub fn run_ws_update() -> (G16cSettingsTrace, usize) {
        let mut runner = Self::new();
        let initial = runner.attach_summary(channel_row(7, "G16c Group", "旧简介", "旧规则"));
        runner.ingest_update_channel();
        let persist_corr = runner.only_channel_persist_corr();
        assert!(
            runner.frames().is_empty(),
            "G16c WS cannot publish a Frame before PersistOk"
        );
        runner
            .harness
            .step(port_reply_ok(persist_corr.raw()), 1_700_001_620_103);
        let terminal = runner.only_summary_frame();
        runner.ingest_update_channel();
        let duplicate_corr = runner.only_channel_persist_corr();
        runner
            .harness
            .step(port_reply_ok(duplicate_corr.raw()), 1_700_001_620_104);
        let duplicate_frame_count = runner.frames().len();
        (
            G16cSettingsTrace {
                initial,
                terminal,
                persist_corr,
            },
            duplicate_frame_count,
        )
    }

    /// 回放业务、传输或本地持久失败并返回可见 Frame 数。
    pub fn run_failure(transport: bool, persist: bool) -> usize {
        let mut runner = Self::new();
        runner.attach_summary(channel_row(7, "G16c Group", "旧简介", "旧规则"));
        runner.harness.step(
            command_json(
                "im_channel_change_info",
                json!({
                    "channel_id":CHANNEL_ID,
                    "purpose":"失败值",
                    "header":"失败规则",
                    "expected_version":7,
                    "req_id":"g16c-failure"
                }),
            ),
            1_700_001_620_200,
        );
        let http_corr = runner.only_http_corr();
        if transport {
            runner.harness.step(
                helix_core::Tick::PortReply {
                    corr: http_corr,
                    outcome: PortOutcome::Err(PortError::Http(503)),
                },
                1_700_001_620_201,
            );
            return runner.frames().len();
        }
        let response = if persist {
            json!({
                "status":"SUCCESS",
                "data":{
                    "channelId":CHANNEL_ID,
                    "displayName":"G16c Group",
                    "purpose":"服务端简介",
                    "header":"服务端规则",
                    "settingVersion":8
                }
            })
        } else {
            json!({
                "status":"FAILED",
                "data":{"code":"CHANNEL_SETTINGS_VERSION_CONFLICT"}
            })
        };
        runner.harness.step(
            port_reply_with_body(
                http_corr.raw(),
                enveloped(Bytes::from(
                    serde_json::to_vec(&response).expect("static G16c failure serializes"),
                )),
            ),
            1_700_001_620_201,
        );
        if persist {
            let persist_corr = runner.only_channel_persist_corr();
            runner.harness.step(
                helix_core::Tick::PortReply {
                    corr: persist_corr,
                    outcome: PortOutcome::Err(PortError::Storage(16)),
                },
                1_700_001_620_202,
            );
        }
        runner.frames().len()
    }

    /// 从 durable settings row 重建 cold attach snapshot。
    pub fn run_recovery() -> InteractionFrameV2 {
        let mut runner = Self::new();
        runner.attach_summary(channel_row(10, "G16c Recovered", "恢复简介", "恢复规则"))
    }

    /// 回放一条 HTTP command 的 authority reply 与本地持久屏障。
    fn run_http_command(
        command: &'static str,
        payload: Value,
        authority: Value,
        expected_version: u64,
    ) -> G16cSettingsTrace {
        let mut runner = Self::new();
        let initial = runner.attach_summary(channel_row(7, "G16c Group", "旧简介", "旧规则"));
        runner
            .harness
            .step(command_json(command, payload), 1_700_001_620_010);
        let http_corr = runner.only_http_corr();
        runner.harness.step(
            port_reply_with_body(
                http_corr.raw(),
                enveloped(Bytes::from(
                    serde_json::to_vec(&json!({"status":"SUCCESS","data":authority}))
                        .expect("static G16c authority serializes"),
                )),
            ),
            1_700_001_620_011,
        );
        let persist_corr = runner.only_channel_persist_corr();
        assert!(
            runner.frames().is_empty(),
            "G16c HTTP cannot publish a Frame before PersistOk"
        );
        runner
            .harness
            .step(port_reply_ok(persist_corr.raw()), 1_700_001_620_012);
        let terminal = runner.only_summary_frame();
        assert_eq!(
            summary_view(&terminal).settings_version,
            expected_version,
            "authority settingVersion reaches typed view"
        );
        G16cSettingsTrace {
            initial,
            terminal,
            persist_corr,
        }
    }

    /// attach channel-summary 并用 durable row 完成首次 snapshot。
    fn attach_summary(&mut self, row: Value) -> InteractionFrameV2 {
        self.harness.step(
            command_json(
                "__helix_request_surface_snapshot",
                json!({
                    "surface":"message.channel-summary",
                    "scope":{"kind":"channel","channelId":CHANNEL_ID}
                }),
            ),
            1_700_001_620_000,
        );
        let corr = self.only_channel_persist_corr();
        self.harness.step(
            port_reply_with_body(
                corr.raw(),
                Bytes::from(
                    serde_json::to_vec(&json!([row])).expect("static G16c storage row serializes"),
                ),
            ),
            1_700_001_620_001,
        );
        self.only_summary_frame()
    }

    /// 注入带 event_seq 与完整 authority 字段的 update_channel。
    fn ingest_update_channel(&mut self) {
        self.harness.step(
            helix_core::Tick::Inbound(InboundBytes(Bytes::from(
                serde_json::to_vec(&json!({
                    "action":"update_channel",
                    "tracing":{"csesTrackId":"g16c-observer"},
                    "data":{
                        "event_seq":62,
                        "channel":{
                            "id":CHANNEL_ID,
                            "displayName":"G16c Observer",
                            "purpose":"观察者简介",
                            "header":"观察者规则",
                            "settingVersion":11,
                            "capabilitiesByRole":{
                                "CREATOR":{"canManageSettings":true}
                            }
                        }
                    }
                }))
                .expect("static G16c WS authority serializes"),
            ))),
            1_700_001_620_102,
        );
    }

    /// 提取当前批次唯一 HTTP correlation。
    fn only_http_corr(&self) -> Correlation {
        only_corr(self.harness.sink.as_slice(), |effect| {
            matches!(effect, Effect::Http { .. })
        })
    }

    /// 提取当前批次唯一 channel Get/Upsert/Update correlation。
    fn only_channel_persist_corr(&self) -> Correlation {
        only_corr(self.harness.sink.as_slice(), |effect| {
            matches!(
                effect,
                Effect::Persist { ops, .. }
                    if ops.iter().any(|op| matches!(
                        op,
                        StorageOp::Get(spec) if spec.table == "channel"
                    ) || matches!(
                        op,
                        StorageOp::BatchUpsert(spec) if spec.table == "channel"
                    ) || matches!(
                        op,
                        StorageOp::BatchUpdate(spec) if spec.table == "channel"
                    ))
            )
        })
    }

    /// 解码当前 Effect 批次中的完整 V2 Frames。
    fn frames(&self) -> Vec<InteractionFrameV2> {
        decode_frames(self.harness.sink.as_slice())
    }

    /// 从 raw compatibility emits 中提取唯一 channel-summary Frame。
    fn only_summary_frame(&self) -> InteractionFrameV2 {
        let mut frames = self.frames().into_iter().filter(|frame| {
            frame
                .mutations
                .iter()
                .any(|mutation| mutation.mutation.surface == SurfaceId::MessageChannelSummary)
        });
        let frame = frames.next().expect("one G16c channel-summary Frame");
        assert!(frames.next().is_none());
        frame
    }
}

/// 逐字段验证 G16c envelope、scope、revision、settings 与 action 合同。
pub fn assert_settings_contract(
    frame: &InteractionFrameV2,
    expected_name: &str,
    expected_purpose: &str,
    expected_header: &str,
    expected_version: u64,
    expected_patch: bool,
) {
    frame.validate().expect("G16c Frame validates");
    assert_eq!(frame.schema_version, 2);
    assert_eq!(frame.mutations.len(), 1);
    assert!(frame.directives.is_empty());
    let mutation = &frame.mutations[0];
    assert_eq!(mutation.mutation.surface, SurfaceId::MessageChannelSummary);
    assert_eq!(
        mutation.mutation.scope,
        SurfaceScope::Channel {
            channel_id: CHANNEL_ID.to_string(),
        }
    );
    assert_eq!(
        mutation.mutation.revision,
        if expected_patch { 2 } else { 1 }
    );
    assert_eq!(mutation.mutation.base_revision, expected_patch.then_some(1));
    let view = summary_view(frame);
    assert_eq!(view.title.text, expected_name);
    assert_eq!(view.avatar.fallback_text, expected_name);
    assert_eq!(view.purpose.as_deref(), Some(expected_purpose));
    assert_eq!(view.notice_summary.as_deref(), Some(expected_header));
    assert_eq!(view.settings_version, expected_version);
    assert!(view
        .actions
        .iter()
        .any(|action| action.kind == "channel-settings" && action.enabled));
}

/// 返回 G16c Frame 内唯一 channel-summary typed view。
fn summary_view(frame: &InteractionFrameV2) -> &helix_im::interaction_frame::ChannelSummaryVm {
    match &frame.mutations[0].mutation.mutation {
        TypedSurfaceMutation::ChannelSummarySnapshot { view } => view,
        TypedSurfaceMutation::ChannelSummaryPatch { patch } => &patch.view,
        other => panic!("expected G16c channel-summary mutation, got {other:?}"),
    }
}

/// 提取 Effect 批次中唯一匹配的 correlation。
fn only_corr(effects: &[Effect], predicate: impl Fn(&Effect) -> bool) -> Correlation {
    let mut correlations = effects
        .iter()
        .filter(|effect| predicate(effect))
        .filter_map(|effect| match effect {
            Effect::Http { corr, .. } | Effect::Persist { corr, .. } => Some(*corr),
            _ => None,
        });
    let corr = correlations.next().expect("matching G16c correlation");
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
        .map(|bytes| InteractionFrameV2::decode(&bytes).expect("producer emits valid G16c V2"))
        .collect()
}

/// 构造 channel storage read model。
fn channel_row(version: u64, display_name: &str, purpose: &str, header: &str) -> Value {
    json!({
        "id":CHANNEL_ID,
        "display_name":display_name,
        "purpose":purpose,
        "header":header,
        "updated_at":version,
        "member_count":2,
        "role":"CREATOR"
    })
}
