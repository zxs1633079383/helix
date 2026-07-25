//! G05 转发消息的 Helix sans-IO 完整回放与 canonical Frame 真源。

use std::collections::BTreeSet;

use bytes::Bytes;
use helix_core::effect::{Correlation, Effect, StorageOp};
use helix_core::tick::{InboundBytes, PortError, PortOutcome, ReplyBytes};
use helix_core::{EffectSink, Module, Tick};
use helix_im::interaction_frame::{
    DialogRowVm, InteractionFrameV2, MessageContentVm, MessageQuickActionVm, MessageVm, PreviewVm,
    SurfaceId, SurfaceScope, TypedSurfaceMutation,
};
use helix_im::module::ImConfig;
use helix_im::ImModule;
use serde_json::{json, Value};

pub const SOURCE_CHANNEL_ID: &str = "chfixx000000000000000001f4";
pub const TARGET_CHANNEL_ID: &str = "chfixx000000000000000001f5";
pub const SOURCE_POST_ID: &str = "srvfix000000000000000001f4";
pub const TARGET_POST_ID: &str = "srvfix000000000000000001f5";
pub const REQUEST_ID: &str = "g05-wdio-forward-1";
pub const MARKER: &str = "g05-forward-fixture-marker";
pub const ACTOR_ID: &str = "g05-wdio-actor";
pub const OBSERVER_ID: &str = "g05-wdio-observer";
pub const TEAM_ID: &str = "g05-wdio-company";
pub const CREATED_AT: i64 = 1_700_000_500_002;
pub const SOURCE_GROUP_NAME: &str = "G05 Source Group";
pub const TARGET_GROUP_NAME: &str = "G05 Target Group";

/// 汇总 G05 成功链的 fresh-group、source、target timeline 与 terminal dialog Frames。
pub struct ForwardScenarioTrace {
    pub group_created: InteractionFrameV2,
    pub source_forwardable: InteractionFrameV2,
    pub target_forwarded: InteractionFrameV2,
    pub target_dialog: InteractionFrameV2,
    pub http_accepted_frame_count: usize,
    pub late_http_frame_count: usize,
}

/// 汇总业务或基础设施失败分支的 Frame 与写入计数。
pub struct ForwardFailureTrace {
    pub group_created: InteractionFrameV2,
    pub source_forwardable: InteractionFrameV2,
    pub terminal_frame_count: usize,
    pub target_message_write_count: usize,
}

/// 汇总 dialog Persist 失败后由后续可信 authority 重放收敛的结果。
pub struct ForwardPersistReplayTrace {
    pub group_created: InteractionFrameV2,
    pub source_forwardable: InteractionFrameV2,
    pub target_forwarded: InteractionFrameV2,
    pub target_dialog_after_replay: InteractionFrameV2,
}

/// 仅通过公开 `ImModule::handle` 驱动 G05 command、PortReply 与 WS authority。
pub struct G05ScenarioRunner {
    module: ImModule,
    sink: EffectSink,
}

impl G05ScenarioRunner {
    /// 创建固定身份、时间和 ID 的确定性生产模块实例。
    fn new() -> Self {
        Self {
            module: ImModule::new(ImConfig {
                api_base_url: "http://g05-wdio.invalid".to_string(),
                default_api_base_url: "http://g05-wdio.invalid".to_string(),
                auth_user_id: ACTOR_ID.to_string(),
                company_id: TEAM_ID.to_string(),
                user_name: "G05 Fixture Actor".to_string(),
                ..ImConfig::default()
            }),
            sink: EffectSink::new(),
        }
    }

    /// 回放正常 individual forward，并验证 WS 先于迟到 HTTP 仍只结算一次。
    pub fn run_happy_ws_first() -> ForwardScenarioTrace {
        let mut runner = Self::new();
        let group_created = runner.create_group_prerequisite();
        let source_forwardable = runner.attach_source_timeline();
        runner.attach_empty_target_timeline();
        let http_corr = runner.start_forward();
        let (target_forwarded, target_dialog) =
            runner.finish_target_authority(1, TARGET_POST_ID, false);
        runner.step(
            ok(
                http_corr,
                http_envelope(json!({
                    "status": "SUCCESS",
                    "data": {"targets": [
                        {"channelId": TARGET_CHANNEL_ID, "status": "accepted"}
                    ]}
                })),
            ),
            CREATED_AT as u64 + 30,
        );
        let late_http_frame_count = runner.frames().len();
        assert_eq!(
            late_http_frame_count, 0,
            "late HTTP admission cannot repeat a terminal Frame"
        );
        runner.step(
            forward_post_frame(1, TARGET_POST_ID),
            CREATED_AT as u64 + 31,
        );
        assert!(
            runner.frames().is_empty(),
            "duplicate authoritative post cannot repeat a terminal Frame"
        );
        ForwardScenarioTrace {
            group_created,
            source_forwardable,
            target_forwarded,
            target_dialog,
            http_accepted_frame_count: 0,
            late_http_frame_count,
        }
    }

    /// 回放 HTTP 全目标业务拒绝，证明 admission failure 不制造 target terminal。
    pub fn run_business_failure() -> ForwardFailureTrace {
        let mut runner = Self::new();
        let group_created = runner.create_group_prerequisite();
        let source_forwardable = runner.attach_source_timeline();
        runner.attach_empty_target_timeline();
        let http_corr = runner.start_forward();
        runner.step(
            ok(
                http_corr,
                http_envelope(json!({
                    "status": "SUCCESS",
                    "data": {"targets": [
                        {"channelId": TARGET_CHANNEL_ID, "status": "failed",
                         "error": "forbidden"}
                    ]}
                })),
            ),
            CREATED_AT as u64 + 20,
        );
        ForwardFailureTrace {
            group_created,
            source_forwardable,
            terminal_frame_count: runner
                .frames()
                .iter()
                .filter(|frame| {
                    frame.mutations.iter().any(|mutation| {
                        mutation.mutation.surface == SurfaceId::MessageLeftDialogList
                    })
                })
                .count(),
            target_message_write_count: count_table_writes(runner.effects(), "message"),
        }
    }

    /// 回放 createPosts transport failure，证明既有 source timeline 保持不变。
    pub fn run_infrastructure_failure() -> ForwardFailureTrace {
        let mut runner = Self::new();
        let group_created = runner.create_group_prerequisite();
        let source_forwardable = runner.attach_source_timeline();
        runner.attach_empty_target_timeline();
        let http_corr = runner.start_forward();
        runner.step(err(http_corr), CREATED_AT as u64 + 20);
        ForwardFailureTrace {
            group_created,
            source_forwardable,
            terminal_frame_count: runner.frames().len(),
            target_message_write_count: count_table_writes(runner.effects(), "message"),
        }
    }

    /// 回放 dialog PersistErr 后新的可信 post authority，证明 ledger 未被失败屏障消费。
    pub fn run_persist_failure_replay() -> ForwardPersistReplayTrace {
        let mut runner = Self::new();
        let group_created = runner.create_group_prerequisite();
        let source_forwardable = runner.attach_source_timeline();
        runner.attach_empty_target_timeline();
        let http_corr = runner.start_forward();
        runner.accept_http(http_corr);
        let (target_forwarded, _) = runner.finish_target_authority(1, TARGET_POST_ID, true);
        let (_, target_dialog_after_replay) =
            runner.finish_target_authority(2, "srvfix000000000000000001f6", false);
        ForwardPersistReplayTrace {
            group_created,
            source_forwardable,
            target_forwarded,
            target_dialog_after_replay,
        }
    }

    /// 依次新建 source 与 target fresh group，并返回 target 的权威 dialog Frame。
    fn create_group_prerequisite(&mut self) -> InteractionFrameV2 {
        self.attach_empty_dialog();
        self.create_group(SOURCE_CHANNEL_ID, SOURCE_GROUP_NAME, -10);
        self.create_group(TARGET_CHANNEL_ID, TARGET_GROUP_NAME, -7)
    }

    /// 建立空 dialog attachment，使后续 create/post 通过完整 Frame 更新会话列表。
    fn attach_empty_dialog(&mut self) {
        self.step(
            command("im_query_dialog_list", json!({})),
            CREATED_AT as u64 - 14,
        );
        let corr = self.only_persist_corr_with(|ops| {
            ops.iter()
                .any(|op| matches!(op, StorageOp::Scan(spec) if spec.table == "channel"))
        });
        self.step(ok(corr, rows(json!([]))), CREATED_AT as u64 - 13);
        assert_eq!(self.only_frame().schema_version, 2);
    }

    /// 通过 production create command/HTTP reply 建立一个权威双用户群。
    fn create_group(
        &mut self,
        channel_id: &str,
        group_name: &str,
        offset: i64,
    ) -> InteractionFrameV2 {
        self.step(
            command(
                "im_create_channel",
                json!({
                    "type": "O",
                    "user_ids": [OBSERVER_ID],
                    "display_name": group_name,
                    "force_create": true,
                    "req_id": format!("{REQUEST_ID}:create:{channel_id}"),
                }),
            ),
            (CREATED_AT + offset) as u64,
        );
        let corr = self.only_http_corr("/channel/create");
        self.step(
            ok(
                corr,
                http_envelope(json!({
                    "status": "SUCCESS",
                    "data": {
                        "id": channel_id,
                        "type": "P",
                        "displayName": group_name,
                        "teamId": TEAM_ID,
                        "createAt": CREATED_AT + offset + 1,
                        "owner": {"id": ACTOR_ID, "role": "OWNER"},
                        "pictureType": "USER",
                        "picture": {"userIds": [ACTOR_ID, OBSERVER_ID]}
                    }
                })),
            ),
            (CREATED_AT + offset + 1) as u64,
        );
        let persisted_tables = self
            .effects()
            .iter()
            .filter_map(|effect| match effect {
                Effect::PersistFire { ops } => Some(ops.as_slice()),
                _ => None,
            })
            .flatten()
            .filter_map(storage_write_table)
            .collect::<BTreeSet<_>>();
        assert!(
            persisted_tables.contains("channel") && persisted_tables.contains("channel_member"),
            "create response must persist channel and member facts; tables={persisted_tables:?}"
        );
        self.only_frame()
    }

    /// 通过 local-first production query 投影可转发 source message 与 opaque action。
    fn attach_source_timeline(&mut self) -> InteractionFrameV2 {
        self.attach_timeline(SOURCE_CHANNEL_ID, json!([source_render_row()]), 0)
    }

    /// 建立 target latest timeline attachment，使 WS authority 触发持久化后 readback。
    fn attach_empty_target_timeline(&mut self) {
        let frame = self.attach_timeline(TARGET_CHANNEL_ID, json!([]), 5);
        assert!(first_message(&frame).is_none());
    }

    /// 执行 local-first query；本地空结果时完成远端空快照。
    fn attach_timeline(
        &mut self,
        channel_id: &str,
        local_rows: Value,
        offset: u64,
    ) -> InteractionFrameV2 {
        self.step(
            command(
                "im_query_messages_by_channel",
                json!({
                    "channel_id": channel_id,
                    "limit": 20,
                    "windowToken": "latest",
                    "req_id": format!("{REQUEST_ID}:query:{channel_id}"),
                }),
            ),
            CREATED_AT as u64 + offset,
        );
        let local_corr = self.only_persist_corr_with(|ops| {
            ops.iter()
                .any(|op| matches!(op, StorageOp::Scan(spec) if spec.table == "message"))
        });
        self.step(
            ok(local_corr, rows(local_rows.clone())),
            CREATED_AT as u64 + offset + 1,
        );
        if self.frames().is_empty() {
            let remote_corr = self.only_http_corr("/posts/getLatestPost");
            self.step(
                ok(
                    remote_corr,
                    http_envelope(json!({"status": "SUCCESS", "data": local_rows})),
                ),
                CREATED_AT as u64 + offset + 2,
            );
            if self.frames().is_empty() {
                let cache_corr = self.only_persist_corr_with(|ops| !ops.is_empty());
                self.step(ok(cache_corr, Bytes::new()), CREATED_AT as u64 + offset + 3);
            }
        }
        self.only_frame()
    }

    /// 提交 canonical relay command 并完成 source message lookup，返回 createPosts correlation。
    fn start_forward(&mut self) -> Correlation {
        self.step(
            command(
                "im_relay_messages",
                json!({
                    "post_ids": [SOURCE_POST_ID],
                    "target_channel_ids": [TARGET_CHANNEL_ID],
                    "mode": "individual",
                    "req_id": REQUEST_ID,
                }),
            ),
            CREATED_AT as u64 + 12,
        );
        let lookup_corr = self.only_persist_corr_with(|ops| {
            ops.iter()
                .any(|op| matches!(op, StorageOp::Get(spec) if spec.table == "message"))
        });
        assert!(
            self.effects()
                .iter()
                .all(|effect| !matches!(effect, Effect::Http { .. })),
            "forward HTTP must wait for source lookup"
        );
        self.step(
            ok(lookup_corr, rows(json!([source_storage_row()]))),
            CREATED_AT as u64 + 13,
        );
        let http_corr = self.only_http_corr("/posts/createPosts");
        let body = self
            .effects()
            .iter()
            .find_map(|effect| match effect {
                Effect::Http { corr, req } if *corr == http_corr => req.body.as_ref(),
                _ => None,
            })
            .and_then(|body| serde_json::from_slice::<Value>(body.as_ref()).ok())
            .expect("forward HTTP body remains inspectable");
        assert_eq!(body["channelIds"], json!([TARGET_CHANNEL_ID]));
        assert_eq!(body["posts"][0]["message"], MARKER);
        assert_eq!(body["posts"][0]["channelId"], "");
        http_corr
    }

    /// 完成全目标 HTTP acceptance，但不把 admission 当作 delivery terminal。
    fn accept_http(&mut self, http_corr: Correlation) {
        self.step(
            ok(
                http_corr,
                http_envelope(json!({
                    "status": "SUCCESS",
                    "data": {"targets": [
                        {"channelId": TARGET_CHANNEL_ID, "status": "accepted"}
                    ]}
                })),
            ),
            CREATED_AT as u64 + 14,
        );
        assert!(
            self.frames().iter().all(|frame| {
                frame.causation_id.as_deref() != Some(REQUEST_ID)
                    || !frame.mutations.iter().any(|mutation| {
                        mutation.mutation.surface == SurfaceId::MessageLeftDialogList
                    })
            }),
            "HTTP acceptance cannot publish terminal dialog confirmation"
        );
    }

    /// 驱动可信 target post、message readback 与 dialog barrier，可选择令 dialog 首次失败。
    fn finish_target_authority(
        &mut self,
        event_seq: u64,
        post_id: &str,
        fail_dialog: bool,
    ) -> (InteractionFrameV2, InteractionFrameV2) {
        self.step(
            forward_post_frame(event_seq, post_id),
            CREATED_AT as u64 + 20 + event_seq,
        );
        assert!(
            self.frames().is_empty(),
            "raw WS authority must wait for durable projection readback"
        );
        assert!(
            self.effects().iter().any(
                |effect| matches!(effect, Effect::PersistFire { ops } if has_table_write(ops, "message"))
            ),
            "trusted post must emit its authoritative message write before readback"
        );
        let dialog_corr = self.only_persist_corr_with(|ops| {
            ops.iter()
                .any(|op| matches!(op, StorageOp::Get(spec) if spec.table == "channel"))
        });
        let read_corr = self.only_persist_corr_with(|ops| {
            ops.iter()
                .any(|op| matches!(op, StorageOp::Scan(spec) if spec.table == "message"))
        });
        self.step(
            ok(
                read_corr,
                rows(json!([target_render_row(event_seq, post_id)])),
            ),
            CREATED_AT as u64 + 22 + event_seq,
        );
        let target_forwarded = self.only_frame();
        if fail_dialog {
            self.step(err(dialog_corr), CREATED_AT as u64 + 23 + event_seq);
            assert!(
                self.frames().is_empty(),
                "dialog PersistErr cannot confirm the forward target"
            );
            return (target_forwarded.clone(), target_forwarded);
        }
        self.step(
            ok(
                dialog_corr,
                rows(json!([target_channel_row(event_seq, post_id)])),
            ),
            CREATED_AT as u64 + 23 + event_seq,
        );
        (target_forwarded, self.only_frame())
    }

    /// 以单调时间推进一个 Tick，并只保留该 step 的 Effects。
    fn step(&mut self, tick: Tick, now_ms: u64) {
        self.sink.clear();
        self.module
            .handle(&tick, now_ms, &mut self.sink)
            .expect("G05 scenario production step must succeed");
    }

    /// 返回当前 step 的有序 Effect 批次。
    fn effects(&self) -> &[Effect] {
        self.sink.as_slice()
    }

    /// 按 URL suffix 提取当前批次唯一 HTTP correlation。
    fn only_http_corr(&self, suffix: &str) -> Correlation {
        let mut matches = self.effects().iter().filter_map(|effect| match effect {
            Effect::Http { corr, req } if req.url.ends_with(suffix) => Some(*corr),
            _ => None,
        });
        let corr = matches.next().expect("expected one HTTP correlation");
        assert!(matches.next().is_none(), "HTTP correlation must be unique");
        corr
    }

    /// 按 StorageOp 语义提取当前批次唯一 Persist correlation。
    fn only_persist_corr_with(&self, predicate: impl Fn(&[StorageOp]) -> bool) -> Correlation {
        let mut matches = self.effects().iter().filter_map(|effect| match effect {
            Effect::Persist { corr, ops } if predicate(ops) => Some(*corr),
            _ => None,
        });
        let corr = matches.next().unwrap_or_else(|| {
            panic!(
                "expected one Persist correlation; effects={:?}",
                self.effects()
            )
        });
        assert!(
            matches.next().is_none(),
            "Persist correlation must be unique"
        );
        corr
    }

    /// 解码当前批次所有 production `im:interaction:frame` Emit。
    fn frames(&self) -> Vec<InteractionFrameV2> {
        self.effects()
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
            .map(|bytes| {
                InteractionFrameV2::decode(&bytes)
                    .expect("production Frame must decode and validate as V2")
            })
            .collect()
    }

    /// 返回当前批次唯一 Frame，拒绝把隐含帧序作为测试假设。
    fn only_frame(&self) -> InteractionFrameV2 {
        let mut frames = self.frames().into_iter();
        let frame = frames.next().expect("expected one InteractionFrameV2");
        assert!(frames.next().is_none(), "Frame output must be unambiguous");
        frame
    }
}

/// 逐字段验证 G05 source timeline 与 opaque forward action。
pub fn assert_source_forwardable_contract(frame: &InteractionFrameV2) {
    frame.validate().expect("source Frame remains valid");
    assert_eq!(frame.schema_version, 2);
    assert_eq!(frame.causation_id, None);
    assert_eq!(frame.mutations.len(), 1);
    let mutation = &frame.mutations[0];
    assert_eq!(mutation.mutation.surface, SurfaceId::MessageTimeline);
    assert_eq!(
        mutation.mutation.scope,
        SurfaceScope::Timeline {
            channel_id: SOURCE_CHANNEL_ID.to_string(),
            window_token: "latest".to_string(),
        }
    );
    let message = first_message(frame).expect("source timeline contains one message");
    assert_eq!(message.id, SOURCE_POST_ID);
    assert!(matches!(
        &message.content,
        MessageContentVm::Text { text } if text == MARKER
    ));
    let action = message
        .actions
        .iter()
        .find(|action| action.kind == "forward")
        .expect("source message exposes forward");
    assert!(action.enabled);
    assert!(!action.action_ref.is_empty());
    assert_eq!(action.surface_epoch, mutation.surface_epoch);
    assert!(frame.directives.is_empty());
}

/// 逐字段验证 target timeline 的权威 post、correlation 与可用 actions。
pub fn assert_target_forwarded_contract(frame: &InteractionFrameV2) {
    frame.validate().expect("target Frame remains valid");
    assert_eq!(frame.schema_version, 2);
    assert_eq!(
        frame.causation_id.as_deref(),
        Some(REQUEST_ID),
        "terminal dialog frame lost forward causation: {frame:#?}"
    );
    let mutation = frame
        .mutations
        .iter()
        .find(|mutation| mutation.mutation.surface == SurfaceId::MessageTimeline)
        .expect("target Frame contains timeline");
    assert_eq!(
        mutation.mutation.scope,
        SurfaceScope::Timeline {
            channel_id: TARGET_CHANNEL_ID.to_string(),
            window_token: "latest".to_string(),
        }
    );
    assert_eq!(mutation.mutation.causation_id.as_deref(), Some(REQUEST_ID));
    let message = first_message(frame).expect("target timeline contains forwarded message");
    assert_eq!(message.id, TARGET_POST_ID);
    assert_eq!(message.channel_id, TARGET_CHANNEL_ID);
    assert_eq!(message.event_seq, 1);
    assert!(matches!(
        &message.content,
        MessageContentVm::Text { text } if text == MARKER
    ));
    assert!(message.actions.iter().any(|action| action.kind == "reply"));
    assert!(message
        .actions
        .iter()
        .any(|action| action.kind == "forward"));
    assert!(message
        .quick_actions
        .iter()
        .any(|action| *action == MessageQuickActionVm::React));
    assert!(frame.directives.is_empty());
}

/// 逐字段验证 final target 的 left-dialog terminal Frame。
pub fn assert_target_dialog_contract(frame: &InteractionFrameV2) {
    assert_target_dialog_contract_for(frame, 1);
}

/// 按 authority event sequence 验证 replay 后的 final target dialog。
pub fn assert_target_dialog_contract_for(frame: &InteractionFrameV2, event_seq: u64) {
    frame.validate().expect("target dialog Frame remains valid");
    assert_eq!(frame.schema_version, 2);
    assert_eq!(
        frame.causation_id.as_deref(),
        Some(REQUEST_ID),
        "terminal dialog frame lost forward causation: {frame:#?}"
    );
    let mutation = frame
        .mutations
        .iter()
        .find(|mutation| mutation.mutation.surface == SurfaceId::MessageLeftDialogList)
        .expect("terminal Frame contains left dialog");
    assert_eq!(mutation.mutation.causation_id.as_deref(), Some(REQUEST_ID));
    let row = first_dialog_row(frame).expect("terminal dialog contains target group");
    assert_eq!(row.channel_id, TARGET_CHANNEL_ID);
    assert_eq!(row.last_activity_at, CREATED_AT + 20 + event_seq as i64);
    assert!(matches!(
        &row.preview,
        PreviewVm::Text { text, .. } if text == MARKER
    ));
    assert!(frame.directives.is_empty());
}

/// 返回 Frame 的第一条 timeline message。
pub fn first_message(frame: &InteractionFrameV2) -> Option<&MessageVm> {
    frame
        .mutations
        .iter()
        .find_map(|mutation| match &mutation.mutation.mutation {
            TypedSurfaceMutation::TimelineSnapshot { view } => view.items.first(),
            TypedSurfaceMutation::TimelinePatch { patch } => patch.view.items.first(),
            _ => None,
        })
}

/// 返回 Frame 的第一条 dialog row。
pub fn first_dialog_row(frame: &InteractionFrameV2) -> Option<&DialogRowVm> {
    frame
        .mutations
        .iter()
        .find_map(|mutation| match &mutation.mutation.mutation {
            TypedSurfaceMutation::DialogListSnapshot { view } => view
                .sections
                .iter()
                .flat_map(|section| &section.items)
                .next(),
            TypedSurfaceMutation::DialogListPatch { patch } => patch
                .view
                .sections
                .iter()
                .flat_map(|section| &section.items)
                .next(),
            _ => None,
        })
}

/// 返回 Frame 中去重后的 surface 名称。
pub fn surface_names(frame: &InteractionFrameV2) -> BTreeSet<&'static str> {
    frame
        .mutations
        .iter()
        .map(|mutation| mutation.mutation.surface.as_str())
        .collect()
}

/// 构造 production AppCommand tick。
fn command(name: &str, payload: Value) -> Tick {
    let payload = serde_json::to_vec(&payload).expect("scenario command payload serializes");
    Tick::Command(
        helix_im::client_api::build_command(name, &payload)
            .expect("public command must normalize through the production client API"),
    )
}

/// 构造成功 PortReply tick。
fn ok(corr: Correlation, body: Bytes) -> Tick {
    Tick::PortReply {
        corr,
        outcome: PortOutcome::Ok(ReplyBytes(body)),
    }
}

/// 构造基础设施失败 PortReply tick。
fn err(corr: Correlation) -> Tick {
    Tick::PortReply {
        corr,
        outcome: PortOutcome::Err(PortError::Http(503)),
    }
}

/// 包装现网同步 HTTP envelope。
fn http_envelope(data: Value) -> Bytes {
    let raw = serde_json::to_vec(&data).expect("HTTP body serializes");
    Bytes::from(
        serde_json::to_vec(&json!({
            "status": 200,
            "headers": {},
            "body": base64(&raw),
        }))
        .expect("HTTP envelope serializes"),
    )
}

/// 以固定字母表编码测试 HTTP body，避免引入新的测试运行时依赖。
fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut encoded = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = u32::from(chunk[0]);
        let b1 = u32::from(chunk.get(1).copied().unwrap_or_default());
        let b2 = u32::from(chunk.get(2).copied().unwrap_or_default());
        let value = (b0 << 16) | (b1 << 8) | b2;
        encoded.push(TABLE[((value >> 18) & 63) as usize] as char);
        encoded.push(TABLE[((value >> 12) & 63) as usize] as char);
        encoded.push(if chunk.len() > 1 {
            TABLE[((value >> 6) & 63) as usize] as char
        } else {
            '='
        });
        encoded.push(if chunk.len() > 2 {
            TABLE[(value & 63) as usize] as char
        } else {
            '='
        });
    }
    encoded
}

/// 编码 driver storage row reply。
fn rows(data: Value) -> Bytes {
    Bytes::from(serde_json::to_vec(&data).expect("storage rows serialize"))
}

/// 构造本地 message lookup 使用的 snake_case durable row。
fn source_storage_row() -> Value {
    json!({
        "id": SOURCE_POST_ID,
        "temporary_id": SOURCE_POST_ID,
        "channel_id": SOURCE_CHANNEL_ID,
        "user_id": ACTOR_ID,
        "message": MARKER,
        "simple_message": MARKER,
        "type": "TEXT",
        "props": "{}",
        "mentions": "[]",
        "viewers": "[\"all\"]",
        "user_snapshot": "{}",
        "create_at": CREATED_AT,
        "send_status": "sent",
    })
}

/// 构造 source timeline projector 使用的 render-ready row。
fn source_render_row() -> Value {
    json!({
        "id": SOURCE_POST_ID,
        "temporaryId": SOURCE_POST_ID,
        "channelId": SOURCE_CHANNEL_ID,
        "userId": ACTOR_ID,
        "createAt": CREATED_AT,
        "message": MARKER,
        "simpleMessage": MARKER,
        "type": "TEXT",
        "sendStatus": "sent",
        "event_seq": 1,
        "props": {},
        "actionCapabilities": {
            "canRevoke": true,
            "canReply": true,
            "canForward": true,
            "canReact": true
        }
    })
}

/// 构造 target timeline readback 使用的 render-ready row。
fn target_render_row(event_seq: u64, post_id: &str) -> Value {
    json!({
        "id": post_id,
        "temporaryId": format!("relay-{REQUEST_ID}-0"),
        "channelId": TARGET_CHANNEL_ID,
        "userId": ACTOR_ID,
        "createAt": CREATED_AT + 20 + event_seq as i64,
        "message": MARKER,
        "simpleMessage": MARKER,
        "type": "TEXT",
        "sendStatus": "sent",
        "event_seq": event_seq,
        "props": {},
        "actionCapabilities": {
            "canRevoke": true,
            "canReply": true,
            "canForward": true,
            "canReact": true
        }
    })
}

/// 构造可信顶层 tracing correlation 的 target WS post。
fn forward_post_frame(event_seq: u64, post_id: &str) -> Tick {
    Tick::Inbound(InboundBytes(
        serde_json::to_vec(&json!({
            "action": "post",
            "tracing": {"csesTrackId": REQUEST_ID},
            "data": {
                "event_seq": event_seq,
                "channelId": TARGET_CHANNEL_ID,
                "post": {
                    "id": post_id,
                    "temporaryId": format!("relay-{REQUEST_ID}-0"),
                    "channelId": TARGET_CHANNEL_ID,
                    "userId": ACTOR_ID,
                    "teamId": TEAM_ID,
                    "createAt": CREATED_AT + 20 + event_seq as i64,
                    "type": "TEXT",
                    "message": MARKER,
                    "simpleMessage": MARKER,
                    "viewers": ["all"],
                    "props": {}
                }
            }
        }))
        .expect("forward WS frame serializes")
        .into(),
    ))
}

/// 构造 dialog 写后 Get 返回的累计 channel row。
fn target_channel_row(event_seq: u64, post_id: &str) -> Value {
    json!({
        "id": TARGET_CHANNEL_ID,
        "team_id": TEAM_ID,
        "display_name": TARGET_GROUP_NAME,
        "type": "P",
        "last_post_at": CREATED_AT + 20 + event_seq as i64,
        "last_root_post_at": CREATED_AT + 20 + event_seq as i64,
        "last_post": MARKER,
        "unread_count": 0,
        "mention_count": 0,
        "urgent_count": 0,
        "unread_post_id": post_id
    })
}

/// 判断一组 StorageOp 是否包含指定表写入。
fn has_table_write(ops: &[StorageOp], table: &str) -> bool {
    ops.iter()
        .filter_map(storage_write_table)
        .any(|candidate| candidate == table)
}

/// 返回 StorageOp 的目标写表，读操作不参与 fresh-group 与 target 写入断言。
fn storage_write_table(op: &StorageOp) -> Option<&'static str> {
    match op {
        StorageOp::BatchUpsert(spec) => Some(spec.table),
        StorageOp::BatchUpdate(spec) => Some(spec.table),
        StorageOp::GuardedBump(spec) => Some(spec.table),
        StorageOp::MonotonicUpsert(spec) => Some(spec.table),
        _ => None,
    }
}

/// 统计当前批次写入指定表的 Effect 数量。
fn count_table_writes(effects: &[Effect], table: &str) -> usize {
    effects
        .iter()
        .filter(|effect| match effect {
            Effect::Persist { ops, .. }
            | Effect::PersistAtomic { ops, .. }
            | Effect::PersistFire { ops } => has_table_write(ops, table),
            _ => false,
        })
        .count()
}
