use bytes::Bytes;
use helix_core::effect::{Correlation, Effect, HttpRequest, StorageOp};
use helix_core::tick::{AppCommand, InboundBytes, PortError, PortOutcome, ReplyBytes};
use helix_core::{EffectSink, Module, Tick};
use helix_im::interaction_frame::{
    InteractionFrameV2, ParticipantReceiptsVm, ReceiptKind, ReceiptStatus, SurfaceId,
    SurfaceScope, TypedSurfaceMutation,
};
use helix_im::module::ImConfig;
use helix_im::query::LocalStoreMode;
use helix_im::state::ChannelId;
use helix_im::ImModule;
use serde_json::{json, Value};

#[path = "../g01_shared/scenario.rs"]
#[allow(dead_code)]
mod shared;

#[allow(unused_imports)]
pub use shared::{assert_group_created_contract_for, sha256_hex, surface_names, SendScenarioSpec};

pub const CHANNEL_ID: &str = "chfixx0000000000000000001e";
pub const POST_ID: &str = "srvfix0000000000000000001e";
pub const REQUEST_ID: &str = "g14-post-read-request-1";
pub const ACTOR_ID: &str = "g14-wdio-actor";
pub const OBSERVER_ID: &str = "g14-wdio-observer";
pub const AUTHOR_ID: &str = "g14-wdio-author";
pub const TEAM_ID: &str = "g14-wdio-company";
pub const GROUP_NAME: &str = "G14 Fixture Group";
pub const CREATED_AT: i64 = 1_700_001_500_000;

/// 返回 G14 fresh-group、单条已读与 receipt snapshot 共用的确定性身份。
pub fn g14_scenario_spec() -> SendScenarioSpec {
    SendScenarioSpec {
        channel_id: CHANNEL_ID,
        server_id: POST_ID,
        request_id: REQUEST_ID,
        marker: "g14-post-read",
        simple_message: "g14-post-read",
        message_type: "TEXT",
        props: json!({}),
        actor_id: ACTOR_ID,
        observer_id: OBSERVER_ID,
        team_id: TEAM_ID,
        created_at: CREATED_AT,
        group_name: GROUP_NAME,
        api_base_url: "http://g14-wdio.invalid",
    }
}

/// 汇总 G14 admission、初始 receipt、持久屏障与终态 Frame。
pub struct PostReadSuccessTrace {
    pub group_created: InteractionFrameV2,
    pub readable_message: InteractionFrameV2,
    pub receipt_viewable_message: InteractionFrameV2,
    pub initial: InteractionFrameV2,
    pub terminal: InteractionFrameV2,
    pub request: HttpRequest,
    pub persist_corr: Correlation,
    pub receipt_corr: Correlation,
}

/// 汇总 G14 不得产生终态 Frame 的失败或重复分支。
pub struct PostReadNoFrameTrace {
    pub initial: InteractionFrameV2,
    pub frame_count: usize,
    pub persist_count: usize,
    pub receipt_http_count: usize,
}

/// 只通过公开 `ImModule::handle` 驱动 G14 command、WS authority、PersistOk 与 receipt 回包。
pub struct G14ScenarioRunner;

impl G14ScenarioRunner {
    /// 回放 fresh group、direct command、WS authority、PersistOk 与 attached receipt refresh。
    pub fn run_success() -> PostReadSuccessTrace {
        let group_created = group_created_frame();
        let readable_message = Self::run_readable_message();
        let receipt_viewable_message = Self::run_receipt_viewable_message();
        let mut replay = Replay::new();
        let initial = replay.attach_receipts("000", 1);
        let command_corr = replay.start_post_read();
        let request = replay.only_http_request("/post/read");
        replay.reply(command_corr, http_success(), 1);
        assert!(
            replay.frames().is_empty(),
            "HTTP success is transport admission only"
        );
        let persist_corr = replay.ingest_post_read_authority("010", 2);
        assert!(
            replay.frames().is_empty(),
            "WS authority must wait for matching PersistOk"
        );
        replay.reply(persist_corr, PortOutcome::Ok(ReplyBytes(Bytes::new())), 3);
        assert!(
            replay.frames().is_empty(),
            "durable authority schedules a receipt query before its typed Frame"
        );
        let receipt_corr = replay.only_http_corr("/post/read/list");
        replay.reply(receipt_corr, receipt_authority("010", 2), 4);
        PostReadSuccessTrace {
            group_created,
            readable_message,
            receipt_viewable_message,
            initial,
            terminal: replay.only_frame(),
            request,
            persist_corr,
            receipt_corr,
        }
    }

    /// 回放 command HTTP failure，证明 transport failure 不改变既有 receipt view。
    pub fn run_command_failure() -> PostReadNoFrameTrace {
        let mut replay = Replay::new();
        let initial = replay.attach_receipts("000", 1);
        let corr = replay.start_post_read();
        replay.reply(corr, PortOutcome::Err(PortError::Http(500)), 1);
        replay.no_frame_trace(initial)
    }

    /// 回放 message.read_bits 写失败，证明 raw event、refresh 与 typed Frame 全部被阻断。
    pub fn run_persist_failure() -> PostReadNoFrameTrace {
        let mut replay = Replay::new();
        let initial = replay.attach_receipts("000", 1);
        let corr = replay.ingest_post_read_authority("010", 2);
        replay.reply(corr, PortOutcome::Err(PortError::Storage(500)), 1);
        replay.no_frame_trace(initial)
    }

    /// 回放已提交 authority 的重复 WS 帧，证明不会重复写入或生成 refresh Frame。
    pub fn run_duplicate_authority() -> PostReadNoFrameTrace {
        let mut replay = Replay::new();
        let initial = replay.attach_receipts("000", 1);
        let persist_corr = replay.ingest_post_read_authority("010", 2);
        replay.reply(persist_corr, PortOutcome::Ok(ReplyBytes(Bytes::new())), 1);
        let receipt_corr = replay.only_http_corr("/post/read/list");
        replay.reply(receipt_corr, receipt_authority("010", 2), 2);
        replay.only_frame();
        replay.step(post_read_inbound("010", 2), CREATED_AT as u64 + 30);
        replay.no_frame_trace(initial)
    }

    /// 回放已经消费的 Persist correlation，证明迟到回包是无副作用 no-op。
    pub fn run_late_persist_reply() -> PostReadNoFrameTrace {
        let mut replay = Replay::new();
        let initial = replay.attach_receipts("000", 1);
        let persist_corr = replay.ingest_post_read_authority("010", 2);
        replay.reply(persist_corr, PortOutcome::Ok(ReplyBytes(Bytes::new())), 1);
        let receipt_corr = replay.only_http_corr("/post/read/list");
        replay.reply(receipt_corr, receipt_authority("010", 2), 2);
        replay.only_frame();
        replay.reply(persist_corr, PortOutcome::Ok(ReplyBytes(Bytes::new())), 3);
        replay.no_frame_trace(initial)
    }

    /// 从远端权威 receipt snapshot 冷 attach，证明恢复不依赖平台缓存或 raw reducer。
    pub fn run_recovery() -> InteractionFrameV2 {
        let mut replay = Replay::new();
        replay.attach_receipts("010", 2)
    }

    /// 从生产 timeline 查询生成带 enabled markRead binding 的可读消息前置 Frame。
    pub fn run_readable_message() -> InteractionFrameV2 {
        Self::run_timeline_message(OBSERVER_ID, "g14-readable-message")
    }

    /// 从生产 timeline 查询生成带 enabled viewReceipt binding 的发送者消息前置 Frame。
    pub fn run_receipt_viewable_message() -> InteractionFrameV2 {
        Self::run_timeline_message(ACTOR_ID, "g14-receipt-viewable-message")
    }

    /// 通过 local scan、HTTP authority 与 cache PersistOk 生成一条生产 timeline Frame。
    fn run_timeline_message(author_id: &str, marker: &str) -> InteractionFrameV2 {
        let mut replay = Replay::new();
        let scan_corr = replay.start_timeline_query();
        replay.reply(
            scan_corr,
            PortOutcome::Ok(ReplyBytes(Bytes::from_static(b"[]"))),
            1,
        );
        let http_corr = replay.only_http_corr("/posts/getLatestPost");
        replay.reply(http_corr, timeline_message_authority(author_id, marker), 2);
        let persist_corr = replay.only_persist_corr_with(|ops| {
            ops.iter()
                .any(|op| matches!(op, StorageOp::BatchUpsert(spec) if spec.table == "message"))
        });
        replay.reply(
            persist_corr,
            PortOutcome::Ok(ReplyBytes(Bytes::new())),
            3,
        );
        replay.only_frame()
    }
}

/// 逐字段验证 G14 pending/completed receipt snapshot 的 scope、revision 与成员集合。
pub fn assert_receipt_contract(
    frame: &InteractionFrameV2,
    expected_revision: u64,
    observer_completed: bool,
) {
    frame.validate().expect("G14 receipt Frame remains valid");
    assert_eq!(frame.schema_version, 2);
    assert_eq!(frame.mutations.len(), 1);
    assert!(frame.directives.is_empty());
    let mutation = &frame.mutations[0];
    assert_eq!(mutation.surface_epoch, 1);
    assert_eq!(mutation.mutation.revision, expected_revision);
    assert_eq!(mutation.mutation.base_revision, None);
    assert_eq!(
        mutation.mutation.scope,
        SurfaceScope::ParticipantReceipts {
            channel_id: CHANNEL_ID.to_string(),
            message_id: POST_ID.to_string(),
            receipt_kind: ReceiptKind::Read,
        }
    );
    let view = receipt_view(frame);
    assert_eq!(view.channel_id, CHANNEL_ID);
    assert_eq!(view.message_id, POST_ID);
    assert_eq!(view.kind, ReceiptKind::Read);
    assert_eq!(view.completed.len(), usize::from(observer_completed));
    assert_eq!(view.pending.len(), usize::from(!observer_completed) + 1);
    let observer = view
        .completed
        .iter()
        .chain(view.pending.iter())
        .find(|row| row.user.user_id == OBSERVER_ID)
        .expect("G14 observer receipt row exists");
    assert_eq!(
        observer.status,
        if observer_completed {
            ReceiptStatus::Completed
        } else {
            ReceiptStatus::Pending
        }
    );
}

/// 返回一份 G14 Frame 中唯一的 participant receipt typed view。
pub fn receipt_view(frame: &InteractionFrameV2) -> &ParticipantReceiptsVm {
    frame
        .mutations
        .iter()
        .find_map(|mutation| match &mutation.mutation.mutation {
            TypedSurfaceMutation::ParticipantReceiptsSnapshot { view } => Some(view),
            TypedSurfaceMutation::ParticipantReceiptsPatch { patch } => Some(&patch.view),
            _ => None,
        })
        .expect("G14 Frame has participant receipt view")
}

/// 验证 PC/Mobile 只能从 Helix Frame 获得单条已读 UI intent 所需的 opaque binding。
pub fn assert_readable_message_contract(frame: &InteractionFrameV2) {
    frame.validate().expect("G14 readable timeline Frame remains valid");
    assert_eq!(frame.schema_version, 2);
    assert_eq!(frame.mutations.len(), 1);
    let mutation = &frame.mutations[0];
    assert_eq!(mutation.mutation.surface, SurfaceId::MessageTimeline);
    let view = match &mutation.mutation.mutation {
        TypedSurfaceMutation::TimelineSnapshot { view } => view,
        TypedSurfaceMutation::TimelinePatch { patch } => &patch.view,
        other => panic!("expected G14 timeline mutation, got {other:?}"),
    };
    assert_eq!(view.channel_id, CHANNEL_ID);
    assert_eq!(view.items.len(), 1);
    let message = &view.items[0];
    assert_eq!(message.id, POST_ID);
    assert_eq!(message.author.user_id, OBSERVER_ID);
    let mark_read = message
        .actions
        .iter()
        .find(|action| action.kind == "markRead")
        .expect("G14 readable message exposes markRead");
    assert!(mark_read.enabled);
    assert!(mark_read.disabled_reason.is_none());
}

/// 验证发送者视角仅暴露 viewReceipt，不允许把自己的消息标记为已读。
pub fn assert_receipt_viewable_message_contract(frame: &InteractionFrameV2) {
    frame
        .validate()
        .expect("G14 receipt-viewable timeline Frame remains valid");
    let mutation = &frame.mutations[0];
    let view = match &mutation.mutation.mutation {
        TypedSurfaceMutation::TimelineSnapshot { view } => view,
        TypedSurfaceMutation::TimelinePatch { patch } => &patch.view,
        other => panic!("expected G14 timeline mutation, got {other:?}"),
    };
    let message = &view.items[0];
    assert_eq!(message.author.user_id, ACTOR_ID);
    let view_receipt = message
        .actions
        .iter()
        .find(|action| action.kind == "viewReceipt")
        .expect("G14 sender message exposes viewReceipt");
    assert!(view_receipt.enabled);
    let mark_read = message
        .actions
        .iter()
        .find(|action| action.kind == "markRead")
        .expect("G14 sender message retains disabled markRead");
    assert!(!mark_read.enabled);
}

/// 用共享生产建群回放生成 G14 fresh-group prerequisite Frame。
fn group_created_frame() -> InteractionFrameV2 {
    let mut runner = shared::G01ScenarioRunner::new_for(g14_scenario_spec());
    runner.create_group_prerequisite()
}

/// 封装一个公开 ImModule 回放实例与当前 step Effects。
struct Replay {
    module: ImModule,
    sink: EffectSink,
}

impl Replay {
    /// 创建带固定 actor/team/api 的 G14 production module。
    fn new() -> Self {
        let mut module = ImModule::new_with_local_store(
            ImConfig {
                api_base_url: "http://g14-wdio.invalid".to_string(),
                default_api_base_url: "http://g14-wdio.invalid".to_string(),
                auth_user_id: ACTOR_ID.to_string(),
                company_id: TEAM_ID.to_string(),
                user_name: "G14 Fixture Actor".to_string(),
                ..ImConfig::default()
            },
            LocalStoreMode::Session,
        );
        module.register_channel(
            ChannelId::from_str(CHANNEL_ID).expect("G14 fixture channel id is valid"),
            0,
        );
        Self {
            module,
            sink: EffectSink::new(),
        }
    }

    /// 执行一个 Tick，并把 sink 重置为该 step 的 Effects。
    fn step(&mut self, tick: Tick, now_ms: u64) {
        self.sink.clear();
        self.module
            .handle(&tick, now_ms, &mut self.sink)
            .expect("G14 production step succeeds");
    }

    /// 通过 opaque Surface resync 与 post/read/list 权威回包 attach receipt snapshot。
    fn attach_receipts(&mut self, read_bits: &str, snapshot_version: u64) -> InteractionFrameV2 {
        self.step(
            command(
                "__helix_request_surface_snapshot",
                json!({
                    "surface": "message.participant-receipts",
                    "scope": {
                        "kind": "participantReceipts",
                        "channelId": CHANNEL_ID,
                        "messageId": POST_ID,
                        "receiptKind": "read"
                    }
                }),
            ),
            CREATED_AT as u64,
        );
        let corr = self.only_http_corr("/post/read/list");
        self.reply(
            corr,
            receipt_authority(read_bits, snapshot_version),
            snapshot_version,
        );
        self.only_frame()
    }

    /// 发起 canonical `im_post_read` 并返回 transport correlation。
    fn start_post_read(&mut self) -> Correlation {
        self.step(
            command(
                "im_post_read",
                json!({"channel_id":CHANNEL_ID,"posts":[POST_ID]}),
            ),
            CREATED_AT as u64 + 10,
        );
        self.only_http_corr("/post/read")
    }

    /// 发起一条本地覆盖充分的 timeline 查询并返回 storage scan correlation。
    fn start_timeline_query(&mut self) -> Correlation {
        self.step(
            command(
                "im_query_messages_by_channel",
                json!({
                    "channel_id": CHANNEL_ID,
                    "limit": 1,
                    "windowToken": "g14-readable-window"
                }),
            ),
            CREATED_AT as u64,
        );
        self.only_persist_corr_with(|ops| {
            ops.iter()
                .any(|op| matches!(op, StorageOp::Scan(spec) if spec.table == "message"))
        })
    }

    /// 注入无 event_seq 的真实 post_read authority 并返回 message persist correlation。
    fn ingest_post_read_authority(
        &mut self,
        read_bits: &str,
        receipt_revision: i64,
    ) -> Correlation {
        self.step(
            post_read_inbound(read_bits, receipt_revision),
            CREATED_AT as u64 + 20,
        );
        self.only_persist_corr_with(|ops| {
            ops.iter().any(|op| {
                matches!(
                    op,
                    StorageOp::BatchUpdate(spec)
                        if spec.table == "message" && spec.key_col == "id"
                )
            })
        })
    }

    /// 回灌一个相关端口结果。
    fn reply(&mut self, corr: Correlation, outcome: PortOutcome, offset_ms: u64) {
        self.step(
            Tick::PortReply { corr, outcome },
            CREATED_AT as u64 + 20 + offset_ms,
        );
    }

    /// 返回当前 step 唯一匹配 URL 的 HTTP correlation。
    fn only_http_corr(&self, suffix: &str) -> Correlation {
        let correlations = self
            .sink
            .as_slice()
            .iter()
            .filter_map(|effect| match effect {
                Effect::Http { corr, req } if req.url.ends_with(suffix) => Some(*corr),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(correlations.len(), 1, "expected one G14 HTTP Effect");
        correlations[0]
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
        assert_eq!(requests.len(), 1, "expected one G14 HTTP request");
        requests[0].clone()
    }

    /// 返回当前 step 唯一满足谓词的相关 Persist correlation。
    fn only_persist_corr_with(&self, predicate: impl Fn(&[StorageOp]) -> bool) -> Correlation {
        let correlations = self
            .sink
            .as_slice()
            .iter()
            .filter_map(|effect| match effect {
                Effect::Persist { corr, ops } if predicate(ops) => Some(*corr),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(correlations.len(), 1, "expected one G14 Persist Effect");
        correlations[0]
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
        assert_eq!(
            frames.len(),
            1,
            "expected one G14 Frame, effects={:?}",
            self.sink.as_slice()
        );
        frames.into_iter().next().expect("one G14 Frame exists")
    }

    /// 汇总当前 step 的无 Frame、Persist 与 receipt HTTP 计数。
    fn no_frame_trace(&self, initial: InteractionFrameV2) -> PostReadNoFrameTrace {
        PostReadNoFrameTrace {
            initial,
            frame_count: self.frames().len(),
            persist_count: self
                .sink
                .as_slice()
                .iter()
                .filter(|effect| {
                    matches!(
                        effect,
                        Effect::Persist { .. }
                            | Effect::PersistAtomic { .. }
                            | Effect::PersistFire { .. }
                    )
                })
                .count(),
            receipt_http_count: self
                .sink
                .as_slice()
                .iter()
                .filter(|effect| {
                    matches!(effect, Effect::Http { req, .. } if req.url.ends_with("/post/read/list"))
                })
                .count(),
        }
    }
}

/// 构造一条指定作者的远端权威消息供生产 timeline projector 决定 action 能力。
fn timeline_message_authority(author_id: &str, marker: &str) -> PortOutcome {
    PortOutcome::Ok(ReplyBytes(http_envelope(json!({
        "status": "SUCCESS",
        "data": [{
            "id": POST_ID,
            "temporaryId": "g14-readable-temporary",
            "channelId": CHANNEL_ID,
            "userId": author_id,
            "type": "TEXT",
            "message": marker,
            "simpleMessage": marker,
            "createAt": CREATED_AT,
            "eventSeq": 1,
            "readMap": "",
            "viewers": ["all"],
            "mentions": []
        }]
    }))))
}

/// 解码 im:interaction:frame 外层事件中的 V2 payload。
fn decode_frame(bytes: &[u8]) -> Option<InteractionFrameV2> {
    let event: Value = serde_json::from_slice(bytes).ok()?;
    if event.get("event").and_then(Value::as_str) != Some("im:interaction:frame") {
        return None;
    }
    InteractionFrameV2::decode(&serde_json::to_vec(event.get("data")?).ok()?).ok()
}

/// 构造生产 command Tick。
fn command(name: &'static str, payload: Value) -> Tick {
    Tick::Command(AppCommand::new(
        name,
        serde_json::to_vec(&payload).expect("G14 command payload serializes"),
    ))
}

/// 构造真实无 event_seq 的 G14 post_read 入站权威帧。
fn post_read_inbound(read_bits: &str, receipt_revision: i64) -> Tick {
    Tick::Inbound(InboundBytes(Bytes::from(
        serde_json::to_vec(&json!({
            "action": "post_read",
            "data": {
                "channelId": CHANNEL_ID,
                "postId": POST_ID,
                "userId": OBSERVER_ID,
                "type": "TEXT",
                "readMap": read_bits,
                "createAt": CREATED_AT,
                "updateAt": receipt_revision
            }
        }))
        .expect("G14 inbound payload serializes"),
    )))
}

/// 构造 command HTTP 成功的 host envelope。
fn http_success() -> PortOutcome {
    PortOutcome::Ok(ReplyBytes(http_envelope(json!({
        "status": "SUCCESS",
        "data": {"accepted": true}
    }))))
}

/// 构造 post/read/list 的完整成员快照权威回包。
fn receipt_authority(read_bits: &str, snapshot_version: u64) -> PortOutcome {
    PortOutcome::Ok(ReplyBytes(http_envelope(json!({
        "status": "SUCCESS",
        "data": [{
            "postId": POST_ID,
            "status": "ready",
            "authorUserId": AUTHOR_ID,
            "snapshotId": format!("g14-snapshot-{snapshot_version}"),
            "snapshotVersion": snapshot_version,
            "memberIds": [AUTHOR_ID, OBSERVER_ID, ACTOR_ID],
            "readBits": read_bits
        }]
    }))))
}

/// 按 host driver ADR-007 形状封装 HTTP body。
fn http_envelope(body: Value) -> Bytes {
    let raw = serde_json::to_vec(&body).expect("G14 HTTP body serializes");
    Bytes::from(
        serde_json::to_vec(&json!({
            "status": 200,
            "headers": {},
            "body": base64(&raw),
        }))
        .expect("G14 HTTP envelope serializes"),
    )
}

/// 用确定性 RFC4648 alphabet 编码测试回包，避免引入平台 HTTP helper。
fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut output = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0];
        let b1 = chunk.get(1).copied().unwrap_or(0);
        let b2 = chunk.get(2).copied().unwrap_or(0);
        output.push(ALPHABET[(b0 >> 2) as usize] as char);
        output.push(ALPHABET[(((b0 & 0x03) << 4) | (b1 >> 4)) as usize] as char);
        output.push(if chunk.len() > 1 {
            ALPHABET[(((b1 & 0x0f) << 2) | (b2 >> 6)) as usize] as char
        } else {
            '='
        });
        output.push(if chunk.len() > 2 {
            ALPHABET[(b2 & 0x3f) as usize] as char
        } else {
            '='
        });
    }
    output
}
