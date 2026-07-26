use bytes::Bytes;
use helix_core::effect::{Correlation, Effect, HttpRequest, StorageOp};
use helix_core::tick::{AppCommand, InboundBytes, PortError, PortOutcome, ReplyBytes};
use helix_core::{EffectSink, Module, Tick};
use helix_im::interaction_frame::{
    DialogListVm, InteractionFrameV2, SurfaceScope, TypedSurfaceMutation, UnreadSummaryVm,
};
use helix_im::module::ImConfig;
use helix_im::ImModule;
use serde_json::{json, Value};

#[path = "../g01_shared/scenario.rs"]
#[allow(dead_code)]
mod shared;

#[allow(unused_imports)]
pub use shared::{assert_group_created_contract_for, sha256_hex, surface_names, SendScenarioSpec};

pub const CHANNEL_ID: &str = "chfixx0000000000000000001d";
pub const ACTOR_ID: &str = "g13-wdio-actor";
pub const OBSERVER_ID: &str = "g13-wdio-observer";
pub const TEAM_ID: &str = "g13-wdio-company";
pub const GROUP_NAME: &str = "G13 Fixture Group";
pub const REQUEST_ID: &str = "g13-channel-view-request-1";
pub const CREATED_AT: i64 = 1_700_001_400_000;

/// 返回 G13 fresh-group、command 与 viewer authority 共用的确定性身份。
pub fn g13_scenario_spec() -> SendScenarioSpec {
    SendScenarioSpec {
        channel_id: CHANNEL_ID,
        server_id: "srvfix0000000000000000001d",
        request_id: REQUEST_ID,
        marker: "g13-channel-read",
        simple_message: "g13-channel-read",
        message_type: "TEXT",
        props: json!({}),
        actor_id: ACTOR_ID,
        observer_id: OBSERVER_ID,
        team_id: TEAM_ID,
        created_at: CREATED_AT,
        group_name: GROUP_NAME,
        api_base_url: "http://g13-wdio.invalid",
    }
}

/// 汇总 G13 command admission、初始未读与 durable 已读 Frame。
pub struct ReadSuccessTrace {
    pub group_created: InteractionFrameV2,
    pub initial: InteractionFrameV2,
    pub terminal: InteractionFrameV2,
    pub request: HttpRequest,
    pub persist_corr: Correlation,
}

/// 汇总不会产生 G13 终态 Frame 的失败或竞态分支。
pub struct ReadNoFrameTrace {
    pub initial: InteractionFrameV2,
    pub frame_count: usize,
}

/// 只通过公开 `ImModule::handle` 驱动 G13 command、WS authority 与 PersistOk。
pub struct G13ScenarioRunner;

impl G13ScenarioRunner {
    /// 回放 fresh group、HTTP admission、update_channel authority 与 durable paired Frame。
    pub fn run_success() -> ReadSuccessTrace {
        let group_created = group_created_frame();
        let mut replay = Replay::new();
        let initial = replay.attach_dialog(initial_dialog_row());
        let http_corr = replay.start_channel_view();
        let request = replay.only_http_request("/channels/view");
        replay.reply(http_corr, http_success(), 1);
        assert!(replay.frames().is_empty(), "HTTP success is admission only");
        let persist_corr = replay.ingest_read_authority();
        assert!(
            replay.frames().is_empty(),
            "update_channel must wait for its correlated PersistOk"
        );
        replay.reply(persist_corr, PortOutcome::Ok(ReplyBytes(Bytes::new())), 3);
        ReadSuccessTrace {
            group_created,
            initial,
            terminal: replay.only_frame(),
            request,
            persist_corr,
        }
    }

    /// 回放 Persist failure，证明 authority 不能在 durable fact 前泄漏终态 Frame。
    pub fn run_persist_failure() -> ReadNoFrameTrace {
        let mut replay = Replay::new();
        let initial = replay.attach_dialog(initial_dialog_row());
        let corr = replay.ingest_read_authority();
        replay.reply(corr, PortOutcome::Err(PortError::Storage(500)), 1);
        ReadNoFrameTrace {
            initial,
            frame_count: replay.frames().len(),
        }
    }

    /// 回放同一 authority 重复到达，证明第二次提交不会推进相同 typed state。
    pub fn run_duplicate_authority() -> ReadNoFrameTrace {
        let mut replay = Replay::new();
        let initial = replay.attach_dialog(initial_dialog_row());
        let first = replay.ingest_read_authority();
        replay.reply(first, PortOutcome::Ok(ReplyBytes(Bytes::new())), 1);
        replay.only_frame();
        let duplicate = replay.ingest_read_authority();
        replay.reply(duplicate, PortOutcome::Ok(ReplyBytes(Bytes::new())), 2);
        ReadNoFrameTrace {
            initial,
            frame_count: replay.frames().len(),
        }
    }

    /// 回放已消费 Persist correlation 的迟到回包，证明不会产生第二个 Frame。
    pub fn run_late_persist_reply() -> ReadNoFrameTrace {
        let mut replay = Replay::new();
        let initial = replay.attach_dialog(initial_dialog_row());
        let corr = replay.ingest_read_authority();
        replay.reply(corr, PortOutcome::Ok(ReplyBytes(Bytes::new())), 1);
        replay.only_frame();
        replay.reply(corr, PortOutcome::Ok(ReplyBytes(Bytes::new())), 2);
        ReadNoFrameTrace {
            initial,
            frame_count: replay.frames().len(),
        }
    }

    /// 从 durable 已读行重新 attach，证明恢复无需平台保留先前 Frame。
    pub fn run_recovery() -> InteractionFrameV2 {
        let mut replay = Replay::new();
        replay.attach_dialog(read_dialog_row())
    }
}

/// 逐字段验证初始 dialog/unread snapshot 的 scope、revision 与未读绝对值。
pub fn assert_initial_contract(frame: &InteractionFrameV2) {
    frame.validate().expect("G13 initial Frame remains valid");
    assert_eq!(frame.schema_version, 2);
    assert_eq!(frame.mutations.len(), 2);
    assert!(frame.directives.is_empty());
    let (dialog, unread) = paired_views(frame, false);
    let row = dialog_row(dialog);
    assert_eq!(row.unread_count, 3);
    assert_eq!(row.mention_count, 1);
    assert_eq!(row.urgent_count, 1);
    assert_eq!(unread.unread_total, 3);
    assert_eq!(unread.mention_total, 1);
    assert_eq!(unread.urgent_total, 1);
    assert_eq!(unread.unread_channel_count, 1);
}

/// 逐字段验证 durable read patch 同帧清零 dialog badges 与 unread aggregate。
pub fn assert_terminal_contract(frame: &InteractionFrameV2) {
    frame.validate().expect("G13 terminal Frame remains valid");
    assert_eq!(frame.schema_version, 2);
    assert_eq!(frame.mutations.len(), 2);
    assert!(frame.directives.is_empty());
    let (dialog, unread) = paired_views(frame, true);
    let row = dialog_row(dialog);
    assert_eq!(row.unread_count, 0);
    assert_eq!(row.mention_count, 0);
    assert_eq!(row.urgent_count, 0);
    assert_eq!(unread.unread_total, 0);
    assert_eq!(unread.mention_total, 0);
    assert_eq!(unread.urgent_total, 0);
    assert_eq!(unread.unread_channel_count, 0);
}

/// 返回 Frame 中 paired dialog/unread view，并校验 scope 与 patch/snapshot 形状。
fn paired_views(frame: &InteractionFrameV2, patch: bool) -> (&DialogListVm, &UnreadSummaryVm) {
    let expected_scope = SurfaceScope::User {
        user_id: ACTOR_ID.to_string(),
        team_id: TEAM_ID.to_string(),
    };
    let mut dialog = None;
    let mut unread = None;
    for mutation in &frame.mutations {
        assert_eq!(mutation.mutation.scope, expected_scope);
        assert_eq!(mutation.surface_epoch, 1);
        match &mutation.mutation.mutation {
            TypedSurfaceMutation::DialogListSnapshot { view } if !patch => dialog = Some(view),
            TypedSurfaceMutation::DialogListPatch { patch: value } if patch => {
                assert_eq!(mutation.mutation.base_revision, Some(1));
                dialog = Some(&value.view);
            }
            TypedSurfaceMutation::UnreadSummarySnapshot { view } if !patch => unread = Some(view),
            TypedSurfaceMutation::UnreadSummaryPatch { patch: value } if patch => {
                assert_eq!(mutation.mutation.base_revision, Some(1));
                unread = Some(&value.view);
            }
            other => panic!("unexpected G13 paired mutation: {other:?}"),
        }
    }
    (
        dialog.expect("G13 Frame has dialog view"),
        unread.expect("G13 Frame has unread view"),
    )
}

/// 返回当前场景唯一的 dialog row。
fn dialog_row(view: &DialogListVm) -> &helix_im::interaction_frame::DialogRowVm {
    view.sections
        .iter()
        .flat_map(|section| section.items.iter())
        .find(|row| row.channel_id == CHANNEL_ID)
        .expect("G13 dialog row exists")
}

/// 用共享生产建群回放生成 G13 fresh-group prerequisite Frame。
fn group_created_frame() -> InteractionFrameV2 {
    let mut runner = shared::G01ScenarioRunner::new_for(g13_scenario_spec());
    runner.create_group_prerequisite()
}

/// 封装一个公开 ImModule 回放实例与当前 step Effects。
struct Replay {
    module: ImModule,
    sink: EffectSink,
}

impl Replay {
    /// 创建带固定 actor/team/api 的 G13 production module。
    fn new() -> Self {
        Self {
            module: ImModule::new(ImConfig {
                api_base_url: "http://g13-wdio.invalid".to_string(),
                default_api_base_url: "http://g13-wdio.invalid".to_string(),
                auth_user_id: ACTOR_ID.to_string(),
                company_id: TEAM_ID.to_string(),
                user_name: "G13 Fixture Actor".to_string(),
                ..ImConfig::default()
            }),
            sink: EffectSink::new(),
        }
    }

    /// 执行一个 Tick，并把 sink 重置为该 step 的 Effects。
    fn step(&mut self, tick: Tick, now_ms: u64) {
        self.sink.clear();
        self.module
            .handle(&tick, now_ms, &mut self.sink)
            .expect("G13 production step succeeds");
    }

    /// 通过 dialog query 与 storage reply attach paired initial snapshot。
    fn attach_dialog(&mut self, row: Value) -> InteractionFrameV2 {
        self.step(
            command("im_query_dialog_list", json!({})),
            CREATED_AT as u64,
        );
        let corr = self.only_persist_corr_with(|ops| {
            ops.iter()
                .any(|op| matches!(op, StorageOp::Scan(scan) if scan.table == "channel"))
        });
        self.reply(
            corr,
            PortOutcome::Ok(ReplyBytes(rows_bytes(json!([row])))),
            1,
        );
        self.only_frame()
    }

    /// 发起 canonical im_channels_view，并返回 HTTP correlation。
    fn start_channel_view(&mut self) -> Correlation {
        self.step(
            command(
                "im_channels_view",
                json!({"channels":[{"id":CHANNEL_ID,"isRoot":true}]}),
            ),
            CREATED_AT as u64 + 10,
        );
        self.only_http_corr("/channels/view")
    }

    /// 注入 server-member-absolute update_channel，并返回其相关 Persist correlation。
    fn ingest_read_authority(&mut self) -> Correlation {
        self.step(read_authority_tick(), CREATED_AT as u64 + 20);
        self.only_persist_corr_with(|ops| {
            ops.iter().any(
                |op| matches!(op, StorageOp::BatchUpsert(spec) if spec.table == "channel_member"),
            )
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
        assert_eq!(correlations.len(), 1, "expected one HTTP Effect");
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
        assert_eq!(requests.len(), 1, "expected one HTTP request");
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
        assert_eq!(correlations.len(), 1, "expected one correlated Persist");
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
        assert_eq!(frames.len(), 1, "expected one G13 Frame");
        frames.into_iter().next().expect("one Frame exists")
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
        serde_json::to_vec(&payload).expect("G13 command serializes"),
    ))
}

/// 构造 driver storage reply bytes。
fn rows_bytes(rows: Value) -> Bytes {
    Bytes::from(serde_json::to_vec(&rows).expect("G13 rows serialize"))
}

/// 构造 channels/view HTTP accepted envelope。
fn http_success() -> PortOutcome {
    let body = serde_json::to_vec(&json!({"status":"SUCCESS","data":null}))
        .expect("G13 HTTP body serializes");
    let envelope = json!({
        "status": 200,
        "headers": {},
        "body": base64(&body),
    });
    PortOutcome::Ok(ReplyBytes(Bytes::from(
        serde_json::to_vec(&envelope).expect("G13 HTTP envelope serializes"),
    )))
}

/// 构造初始未读 channel storage row。
fn initial_dialog_row() -> Value {
    dialog_row_value(3, 1, 1)
}

/// 构造 durable 已读 channel storage row。
fn read_dialog_row() -> Value {
    dialog_row_value(0, 0, 0)
}

/// 构造 dialog query 所需的最小 render-ready storage row。
fn dialog_row_value(unread: i64, mention: i64, urgent: i64) -> Value {
    json!({
        "id": CHANNEL_ID,
        "display_name": GROUP_NAME,
        "type": "P",
        "team_id": TEAM_ID,
        "member_count": 2,
        "unread_count": unread,
        "mention_count": mention,
        "urgent_count": urgent,
        "last_post_at": CREATED_AT - 10,
        "last_post": serde_json::json!({
            "id": "srvfix0000000000000000000d",
            "channelId": CHANNEL_ID,
            "userId": OBSERVER_ID,
            "message": "G13 unread seed",
            "type": "TEXT",
            "createAt": CREATED_AT - 10
        }).to_string(),
        "picture_type": "USER",
        "picture": serde_json::json!({"userIds":[ACTOR_ID,OBSERVER_ID]}).to_string()
    })
}

/// 构造 G13 当前 viewer 的绝对已读 update_channel authority。
fn read_authority_tick() -> Tick {
    Tick::Inbound(InboundBytes(Bytes::from(
        serde_json::to_vec(&json!({
            "action": "update_channel",
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
        .expect("G13 update_channel serializes"),
    )))
}

/// 编码测试 HTTP envelope 所需的标准 Base64。
fn base64(input: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let a = chunk[0];
        let b = *chunk.get(1).unwrap_or(&0);
        let c = *chunk.get(2).unwrap_or(&0);
        out.push(TABLE[(a >> 2) as usize] as char);
        out.push(TABLE[(((a & 0x03) << 4) | (b >> 4)) as usize] as char);
        out.push(if chunk.len() > 1 {
            TABLE[(((b & 0x0f) << 2) | (c >> 6)) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            TABLE[(c & 0x3f) as usize] as char
        } else {
            '='
        });
    }
    out
}
