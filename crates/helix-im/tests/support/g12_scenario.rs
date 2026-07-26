use bytes::Bytes;
use helix_core::effect::{Correlation, Effect, HttpRequest};
use helix_core::tick::{AppCommand, PortError, PortOutcome, ReplyBytes};
use helix_core::{EffectSink, Module, Tick};
use helix_im::interaction_frame::{
    InteractionFrameV2, SurfaceId, SurfaceScope, TypedSurfaceMutation,
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

pub const CHANNEL_ID: &str = "chfixx0000000000000000001c";
pub const ROOT_ID: &str = "srvfix0000000000000000001c";
pub const FIRST_REPLY_ID: &str = "srvfix0000000000000000002c";
pub const SECOND_REPLY_ID: &str = "srvfix0000000000000000003c";
pub const BRANCH_REPLY_ID: &str = "srvfix0000000000000000004c";
pub const ACTOR_ID: &str = "g12-wdio-actor";
pub const OBSERVER_ID: &str = "g12-wdio-observer";
pub const TEAM_ID: &str = "g12-wdio-company";
pub const CREATED_AT: i64 = 1_700_001_300_000;
pub const GROUP_NAME: &str = "G12 Fixture Group";
pub const SNAPSHOT_REQUEST_ID: &str = "g12-replies-request-1";
pub const BRANCH_REQUEST_ID: &str = "g12-branch-request-1";

/// 返回 G12 fresh-group 与 reply-thread correlation 共用的确定性身份。
pub fn g12_scenario_spec() -> SendScenarioSpec {
    SendScenarioSpec {
        channel_id: CHANNEL_ID,
        server_id: ROOT_ID,
        request_id: SNAPSHOT_REQUEST_ID,
        marker: "g12-thread-root",
        simple_message: "g12-thread-root",
        message_type: "TEXT",
        props: json!({}),
        actor_id: ACTOR_ID,
        observer_id: OBSERVER_ID,
        team_id: TEAM_ID,
        created_at: CREATED_AT,
        group_name: GROUP_NAME,
        api_base_url: "http://g12-wdio.invalid",
    }
}

/// 汇总 G12 fresh group、thread snapshot、branch patch 与 production HTTP。
pub struct ReplySuccessTrace {
    pub group_created: InteractionFrameV2,
    pub snapshot: InteractionFrameV2,
    pub branch: InteractionFrameV2,
    pub snapshot_request: HttpRequest,
    pub branch_request: HttpRequest,
    pub snapshot_effect_count: usize,
    pub snapshot_raw_emit_count: usize,
    pub snapshot_persist_count: usize,
}

/// 汇总失败、竞态与恢复场景中的可观察 Frame 数量。
pub struct ReplyNoFrameTrace {
    pub frame_count: usize,
    pub recovered: Option<InteractionFrameV2>,
}

/// 仅通过公开 `ImModule::handle` 驱动 G12 production command 与 Frame。
pub struct G12ScenarioRunner;

impl G12ScenarioRunner {
    /// 回放 fresh group、reply snapshot 与 attached branch patch。
    pub fn run_success() -> ReplySuccessTrace {
        let group_created = group_created_frame();
        let mut replay = Replay::new();
        let snapshot_corr = replay.start_snapshot();
        let snapshot_request = replay.only_http_request("/posts/getReplies");
        replay.reply(
            snapshot_corr,
            PortOutcome::Ok(ReplyBytes(http_envelope(snapshot_body(
                CHANNEL_ID, ROOT_ID,
            )))),
            1,
        );
        let snapshot_effect_count = replay.sink.as_slice().len();
        let snapshot_raw_emit_count = replay.raw_emit_count();
        let snapshot_persist_count = replay.persist_count();
        let snapshot = replay.only_frame();
        let branch_corr = replay.start_branch();
        let branch_request = replay.only_http_request("/posts/getReplyBranch");
        replay.reply(
            branch_corr,
            PortOutcome::Ok(ReplyBytes(http_envelope(branch_body(CHANNEL_ID, ROOT_ID)))),
            2,
        );
        ReplySuccessTrace {
            group_created,
            snapshot,
            branch: replay.only_frame(),
            snapshot_request,
            branch_request,
            snapshot_effect_count,
            snapshot_raw_emit_count,
            snapshot_persist_count,
        }
    }

    /// 回放 attached thread 的空分支成功，证明 command 仍以 causal Frame 结算。
    pub fn run_empty_branch() -> InteractionFrameV2 {
        let mut replay = Replay::new();
        let snapshot_corr = replay.start_snapshot();
        replay.reply(
            snapshot_corr,
            PortOutcome::Ok(ReplyBytes(http_envelope(snapshot_body(
                CHANNEL_ID, ROOT_ID,
            )))),
            1,
        );
        replay.only_frame();
        let branch_corr = replay.start_branch();
        replay.reply(
            branch_corr,
            PortOutcome::Ok(ReplyBytes(http_envelope(empty_branch_body()))),
            2,
        );
        replay.only_frame()
    }

    /// 回放 HTTP failure，证明 raw reply 与伪成功 Frame 均不会泄漏。
    pub fn run_http_failure() -> ReplyNoFrameTrace {
        let mut replay = Replay::new();
        let corr = replay.start_snapshot();
        replay.reply(corr, PortOutcome::Err(PortError::Http(500)), 1);
        ReplyNoFrameTrace {
            frame_count: replay.frames().len(),
            recovered: None,
        }
    }

    /// 回放 mixed-channel authority，证明整批关系 fail-closed。
    pub fn run_mixed_authority() -> ReplyNoFrameTrace {
        let mut replay = Replay::new();
        let corr = replay.start_snapshot();
        replay.reply(
            corr,
            PortOutcome::Ok(ReplyBytes(http_envelope(snapshot_body(
                "chfixx000000000000000000ff",
                ROOT_ID,
            )))),
            1,
        );
        ReplyNoFrameTrace {
            frame_count: replay.frames().len(),
            recovered: None,
        }
    }

    /// 回放 consumed correlation 的重复 reply，证明不会产生第二个 Frame。
    pub fn run_duplicate_reply() -> ReplyNoFrameTrace {
        let mut replay = Replay::new();
        let corr = replay.start_snapshot();
        let outcome = PortOutcome::Ok(ReplyBytes(http_envelope(snapshot_body(
            CHANNEL_ID, ROOT_ID,
        ))));
        replay.reply(corr, outcome.clone(), 1);
        let recovered = replay.only_frame();
        replay.reply(corr, outcome, 2);
        ReplyNoFrameTrace {
            frame_count: replay.frames().len(),
            recovered: Some(recovered),
        }
    }

    /// 回放 detached branch 后重新查询 snapshot，证明恢复不依赖平台缓存。
    pub fn run_detached_recovery() -> ReplyNoFrameTrace {
        let mut replay = Replay::new();
        let branch_corr = replay.start_branch();
        replay.reply(
            branch_corr,
            PortOutcome::Ok(ReplyBytes(http_envelope(branch_body(CHANNEL_ID, ROOT_ID)))),
            1,
        );
        let detached_count = replay.frames().len();
        let snapshot_corr = replay.start_snapshot();
        replay.reply(
            snapshot_corr,
            PortOutcome::Ok(ReplyBytes(http_envelope(snapshot_body(
                CHANNEL_ID, ROOT_ID,
            )))),
            2,
        );
        ReplyNoFrameTrace {
            frame_count: detached_count,
            recovered: Some(replay.only_frame()),
        }
    }
}

/// 逐字段验证 G12 snapshot 的 envelope、scope、顺序、节点与分页。
pub fn assert_snapshot_contract(frame: &InteractionFrameV2) {
    frame.validate().expect("G12 snapshot Frame remains valid");
    assert_eq!(frame.schema_version, 2);
    assert_eq!(frame.causation_id.as_deref(), Some(SNAPSHOT_REQUEST_ID));
    assert_eq!(frame.mutations.len(), 1);
    let mutation = &frame.mutations[0];
    assert_eq!(mutation.mutation.surface, SurfaceId::MessageThreadDetail);
    assert_eq!(
        mutation.mutation.scope,
        SurfaceScope::Thread {
            channel_id: CHANNEL_ID.to_string(),
            root_message_id: ROOT_ID.to_string(),
        }
    );
    assert_eq!(mutation.mutation.base_revision, None);
    assert_eq!(mutation.mutation.revision, 1);
    assert_eq!(mutation.surface_epoch, 1);
    let TypedSurfaceMutation::ThreadDetailSnapshot { view } = &mutation.mutation.mutation else {
        panic!("G12 initial mutation must be a complete thread snapshot");
    };
    assert_eq!(view.channel_id, CHANNEL_ID);
    assert_eq!(view.root.id, ROOT_ID);
    assert_eq!(
        view.ordered_reply_ids,
        vec![FIRST_REPLY_ID.to_string(), SECOND_REPLY_ID.to_string()]
    );
    assert_eq!(view.reply_count, 2);
    assert_eq!(view.nodes.len(), 2);
    assert_eq!(view.page.window_token, format!("thread:{ROOT_ID}"));
    assert!(!view.page.has_more);
    assert!(frame.directives.is_empty());
}

/// 逐字段验证 G12 branch patch 的 revision、causation 与新增节点。
pub fn assert_branch_contract(frame: &InteractionFrameV2) {
    frame.validate().expect("G12 branch Frame remains valid");
    assert_eq!(frame.schema_version, 2);
    assert_eq!(frame.causation_id.as_deref(), Some(BRANCH_REQUEST_ID));
    assert_eq!(frame.mutations.len(), 1);
    let mutation = &frame.mutations[0];
    assert_eq!(mutation.mutation.base_revision, Some(1));
    assert_eq!(mutation.mutation.revision, 2);
    let TypedSurfaceMutation::ThreadDetailPatch { patch } = &mutation.mutation.mutation else {
        panic!("G12 branch mutation must be a complete thread patch");
    };
    assert_eq!(
        patch.view.ordered_reply_ids,
        vec![
            FIRST_REPLY_ID.to_string(),
            SECOND_REPLY_ID.to_string(),
            BRANCH_REPLY_ID.to_string(),
        ]
    );
    assert_eq!(patch.view.reply_count, 3);
    assert!(patch.view.nodes.contains_key(BRANCH_REPLY_ID));
}

/// 逐字段验证空 branch 仍推进 revision、保留已有节点并携带 command causation。
pub fn assert_empty_branch_contract(frame: &InteractionFrameV2) {
    frame
        .validate()
        .expect("G12 empty branch Frame remains valid");
    assert_eq!(frame.schema_version, 2);
    assert_eq!(frame.causation_id.as_deref(), Some(BRANCH_REQUEST_ID));
    assert_eq!(frame.mutations.len(), 1);
    let mutation = &frame.mutations[0];
    assert_eq!(mutation.mutation.base_revision, Some(1));
    assert_eq!(mutation.mutation.revision, 2);
    let TypedSurfaceMutation::ThreadDetailPatch { patch } = &mutation.mutation.mutation else {
        panic!("G12 empty branch must settle with a complete thread patch");
    };
    assert_eq!(
        patch.view.ordered_reply_ids,
        vec![FIRST_REPLY_ID.to_string(), SECOND_REPLY_ID.to_string()]
    );
    assert_eq!(patch.view.reply_count, 2);
    assert_eq!(patch.view.nodes.len(), 2);
    assert!(!patch.view.nodes.contains_key(BRANCH_REPLY_ID));
}

/// 从共享生产建群回放生成 G12 fresh-group Frame。
fn group_created_frame() -> InteractionFrameV2 {
    let spec = g12_scenario_spec();
    let mut runner = shared::G01ScenarioRunner::new_for(spec.clone());
    let frame = runner.create_group_prerequisite();
    assert_group_created_contract_for(&frame, &spec);
    frame
}

/// 持有单个 production ImModule、当前 Effect batch 与确定性时钟。
struct Replay {
    module: ImModule,
    sink: EffectSink,
    now_ms: u64,
}

impl Replay {
    /// 创建带生产身份与 G12 channel 注册的 session-store 回放器。
    fn new() -> Self {
        let mut module = ImModule::new_with_local_store(
            ImConfig {
                api_base_url: "http://g12-wdio.invalid".to_string(),
                default_api_base_url: "http://g12-wdio.invalid".to_string(),
                auth_user_id: ACTOR_ID.to_string(),
                company_id: TEAM_ID.to_string(),
                user_name: "G12 Actor".to_string(),
                ..ImConfig::default()
            },
            LocalStoreMode::Session,
        );
        module.register_channel(
            ChannelId::from_str(CHANNEL_ID).expect("G12 channel id fixture is valid"),
            0,
        );
        Self {
            module,
            sink: EffectSink::new(),
            now_ms: CREATED_AT as u64,
        }
    }

    /// 提交 canonical snapshot command 并返回 `/posts/getReplies` correlation。
    fn start_snapshot(&mut self) -> Correlation {
        self.step(Tick::Command(AppCommand::new(
            "im_get_replies",
            serde_json::to_vec(&json!({
                "reply_id": ROOT_ID,
                "req_id": SNAPSHOT_REQUEST_ID,
                "channel_id": CHANNEL_ID,
                "root_message_id": ROOT_ID,
                "page_number": 1,
                "page_size": 20,
            }))
            .expect("G12 snapshot command serializes"),
        )));
        self.only_http_corr("/posts/getReplies")
    }

    /// 提交 canonical branch command 并返回 `/posts/getReplyBranch` correlation。
    fn start_branch(&mut self) -> Correlation {
        self.step(Tick::Command(AppCommand::new(
            "im_get_reply_branch",
            serde_json::to_vec(&json!({
                "reply_first_level_id": FIRST_REPLY_ID,
                "req_id": BRANCH_REQUEST_ID,
                "channel_id": CHANNEL_ID,
                "root_message_id": ROOT_ID,
                "page_number": 1,
                "page_size": 20,
            }))
            .expect("G12 branch command serializes"),
        )));
        self.only_http_corr("/posts/getReplyBranch")
    }

    /// 回灌一个 port outcome 并推进确定性时钟。
    fn reply(&mut self, corr: Correlation, outcome: PortOutcome, offset_ms: u64) {
        self.now_ms = (CREATED_AT as u64) + offset_ms;
        self.step(Tick::PortReply { corr, outcome });
    }

    /// 通过公开 Module trait 执行一步并仅保留该步 Effect。
    fn step(&mut self, tick: Tick) {
        self.sink.clear();
        self.module
            .handle(&tick, self.now_ms, &mut self.sink)
            .expect("G12 production handle succeeds");
    }

    /// 提取当前唯一匹配路径的 HTTP correlation。
    fn only_http_corr(&self, path: &str) -> Correlation {
        let corrs = self
            .sink
            .as_slice()
            .iter()
            .filter_map(|effect| match effect {
                Effect::Http { corr, req } if req.url.ends_with(path) => Some(*corr),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(corrs.len(), 1, "expected exactly one HTTP for {path}");
        corrs[0]
    }

    /// 克隆当前唯一匹配路径的 production HTTP request。
    fn only_http_request(&self, path: &str) -> HttpRequest {
        self.sink
            .as_slice()
            .iter()
            .find_map(|effect| match effect {
                Effect::Http { req, .. } if req.url.ends_with(path) => Some(req.clone()),
                _ => None,
            })
            .unwrap_or_else(|| panic!("missing HTTP request for {path}"))
    }

    /// 解码当前 batch 中唯一 production InteractionFrameV2。
    fn only_frame(&self) -> InteractionFrameV2 {
        let frames = self.frames();
        assert_eq!(frames.len(), 1, "expected exactly one G12 Frame");
        frames.into_iter().next().expect("one frame exists")
    }

    /// 解码当前 batch 的全部 production InteractionFrameV2。
    fn frames(&self) -> Vec<InteractionFrameV2> {
        self.sink
            .as_slice()
            .iter()
            .filter_map(|effect| match effect {
                Effect::Emit { event } => {
                    let envelope: Value =
                        serde_json::from_slice(event.0.as_ref()).expect("G12 event envelope");
                    (envelope["event"] == "im:interaction:frame").then(|| {
                        InteractionFrameV2::decode(
                            &serde_json::to_vec(&envelope["data"]).expect("G12 frame data"),
                        )
                        .expect("G12 emitted Frame decodes")
                    })
                }
                _ => None,
            })
            .collect()
    }

    /// 统计当前 batch 中非 InteractionFrame 的 raw DomainEvent。
    fn raw_emit_count(&self) -> usize {
        self.sink
            .as_slice()
            .iter()
            .filter(|effect| match effect {
                Effect::Emit { event } => serde_json::from_slice::<Value>(event.0.as_ref())
                    .ok()
                    .is_some_and(|envelope| envelope["event"] != "im:interaction:frame"),
                _ => false,
            })
            .count()
    }

    /// 统计当前 batch 中持久化 Effect，读-only G12 必须保持为零。
    fn persist_count(&self) -> usize {
        self.sink
            .as_slice()
            .iter()
            .filter(|effect| matches!(effect, Effect::Persist { .. } | Effect::PersistFire { .. }))
            .count()
    }
}

/// 构造 getReplies 的 root + 两条一级回复 authority body。
fn snapshot_body(channel_id: &str, root_id: &str) -> Bytes {
    Bytes::from(
        serde_json::to_vec(&json!({
            "rootPost": post(root_id, channel_id, ACTOR_ID, "g12-thread-root", None, None),
            "replies": [
                post(FIRST_REPLY_ID, channel_id, OBSERVER_ID, "g12-first-reply", Some(root_id), Some(FIRST_REPLY_ID)),
                post(SECOND_REPLY_ID, channel_id, ACTOR_ID, "g12-second-reply", Some(root_id), Some(SECOND_REPLY_ID)),
            ],
            "hasMore": false,
        }))
        .expect("G12 snapshot body serializes"),
    )
}

/// 构造 getReplyBranch 的新增分支 authority body。
fn branch_body(channel_id: &str, root_id: &str) -> Bytes {
    Bytes::from(
        serde_json::to_vec(&json!({
            "data": [
                post(
                    BRANCH_REPLY_ID,
                    channel_id,
                    OBSERVER_ID,
                    "g12-branch-reply",
                    Some(root_id),
                    Some(FIRST_REPLY_ID),
                )
            ],
            "hasMore": false,
        }))
        .expect("G12 branch body serializes"),
    )
}

/// 构造成功但无新增 reply 的 getReplyBranch authority body。
fn empty_branch_body() -> Bytes {
    Bytes::from(
        serde_json::to_vec(&json!({
            "data": [],
            "hasMore": false,
        }))
        .expect("G12 empty branch body serializes"),
    )
}

/// 构造一个 viewer-visible server Post。
fn post(
    id: &str,
    channel_id: &str,
    user_id: &str,
    text: &str,
    reply_root_id: Option<&str>,
    reply_first_level_id: Option<&str>,
) -> Value {
    json!({
        "id": id,
        "channelId": channel_id,
        "userId": user_id,
        "type": "TEXT",
        "message": text,
        "simpleMessage": text,
        "replyId": reply_root_id.unwrap_or(""),
        "replyRootId": reply_root_id.unwrap_or(""),
        "replyFirstLevelId": reply_first_level_id.unwrap_or(""),
        "viewers": ["all"],
        "mentions": [],
        "createAt": CREATED_AT,
    })
}

/// 将裸 Go body 包成 host 实际回灌的 ADR-007 HTTP envelope。
fn http_envelope(body: Bytes) -> Bytes {
    Bytes::from(
        serde_json::to_vec(&json!({
            "status": 200,
            "headers": {},
            "body": base64_std(body.as_ref()),
        }))
        .expect("G12 HTTP envelope serializes"),
    )
}

/// 以标准表编码测试 HTTP envelope body。
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
