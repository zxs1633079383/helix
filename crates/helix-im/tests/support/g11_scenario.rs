use bytes::Bytes;
use helix_core::effect::{Correlation, Effect, HttpRequest, StorageOp};
use helix_core::tick::{AppCommand, PortError, PortOutcome, ReplyBytes};
use helix_core::{EffectSink, Module, Tick};
use helix_im::interaction_frame::{
    InteractionFrameV2, SurfaceId, SurfaceScope, TimelineAnchorMode, TypedSurfaceMutation,
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

pub const CHANNEL_ID: &str = "chfixx0000000000000000001b";
pub const WINDOW_TOKEN: &str = "g11-locate-window";
pub const ACTOR_ID: &str = "g11-wdio-actor";
pub const OBSERVER_ID: &str = "g11-wdio-observer";
pub const TEAM_ID: &str = "g11-wdio-company";
pub const CREATED_AT: i64 = 1_700_001_200_000;
pub const GROUP_NAME: &str = "G11 Fixture Group";
pub const REQUEST_ID: &str = "g11-locate-request-1";
pub const TARGET_ID: &str = "srvfix0000000000000000000b";
pub const TARGET_MARKER: &str = "g11-locate-target";

/// 返回 G11 fresh-group、timeline 与 locate correlation 共用的确定性身份。
pub fn g11_scenario_spec() -> SendScenarioSpec {
    SendScenarioSpec {
        channel_id: CHANNEL_ID,
        server_id: TARGET_ID,
        request_id: REQUEST_ID,
        marker: TARGET_MARKER,
        simple_message: TARGET_MARKER,
        message_type: "TEXT",
        props: json!({}),
        actor_id: ACTOR_ID,
        observer_id: OBSERVER_ID,
        team_id: TEAM_ID,
        created_at: CREATED_AT,
        group_name: GROUP_NAME,
        api_base_url: "http://g11-wdio.invalid",
    }
}

/// 汇总成功 locate 的前置 Frame、最终 Frame 与 production HTTP 请求。
pub struct LocateSuccessTrace {
    pub group_created: InteractionFrameV2,
    pub initial: InteractionFrameV2,
    pub located: InteractionFrameV2,
    pub request: HttpRequest,
}

/// 汇总失败或竞态场景中禁止泄漏的 Effect 数量。
pub struct LocateNoFrameTrace {
    pub initial: InteractionFrameV2,
    pub effect_count: usize,
    pub retry: Option<InteractionFrameV2>,
}

/// 仅通过公开 `ImModule::handle` 驱动 G11 production command 与 Frame。
pub struct G11ScenarioRunner;

impl G11ScenarioRunner {
    /// 回放 fresh group、G10 bounded timeline 与成功 HTTP locate authority。
    pub fn run_success() -> LocateSuccessTrace {
        let group_created = group_created_frame();
        let mut replay = Replay::new();
        let initial = replay.attach_timeline();
        let locate_corr = replay.start_locate(TARGET_ID);
        let request = replay.only_http_request("/posts/get");
        replay.reply(
            locate_corr,
            PortOutcome::Ok(ReplyBytes(http_envelope(locate_body(
                CHANNEL_ID, TARGET_ID,
            )))),
            10,
        );
        LocateSuccessTrace {
            group_created,
            initial,
            located: replay.only_frame(),
            request,
        }
    }

    /// 回放 transport failure，证明既有 timeline 不被 raw event 或伪成功 Frame 改写。
    pub fn run_http_failure() -> LocateNoFrameTrace {
        let mut replay = Replay::new();
        let initial = replay.attach_timeline();
        let corr = replay.start_locate(TARGET_ID);
        replay.reply(corr, PortOutcome::Err(PortError::Http(500)), 10);
        LocateNoFrameTrace {
            initial,
            effect_count: replay.effects().len(),
            retry: None,
        }
    }

    /// 回放一次失败后重新发起合法请求，证明失败不污染下一 correlation。
    pub fn run_recovery() -> LocateNoFrameTrace {
        let mut replay = Replay::new();
        let initial = replay.attach_timeline();
        let failed = replay.start_locate(TARGET_ID);
        replay.reply(failed, PortOutcome::Err(PortError::Http(500)), 10);
        let retry_corr = replay.start_locate(TARGET_ID);
        replay.reply(
            retry_corr,
            PortOutcome::Ok(ReplyBytes(http_envelope(locate_body(
                CHANNEL_ID, TARGET_ID,
            )))),
            11,
        );
        LocateNoFrameTrace {
            initial,
            effect_count: 0,
            retry: Some(replay.only_frame()),
        }
    }

    /// 回放已消费 correlation 的重复 reply，证明不会产生第二个 Frame。
    pub fn run_duplicate_reply() -> LocateNoFrameTrace {
        let mut replay = Replay::new();
        let initial = replay.attach_timeline();
        let corr = replay.start_locate(TARGET_ID);
        let reply = PortOutcome::Ok(ReplyBytes(http_envelope(locate_body(
            CHANNEL_ID, TARGET_ID,
        ))));
        replay.reply(corr, reply.clone(), 10);
        let retry = replay.only_frame();
        replay.reply(corr, reply, 11);
        LocateNoFrameTrace {
            initial,
            effect_count: replay.effects().len(),
            retry: Some(retry),
        }
    }

    /// 回放目标未在 attached window 的请求，证明端侧不能插入孤立消息。
    pub fn run_absent_target() -> LocateNoFrameTrace {
        let mut replay = Replay::new();
        let initial = replay.attach_timeline();
        let corr = replay.start_locate("srvfix000000000000000000ff");
        replay.reply(
            corr,
            PortOutcome::Ok(ReplyBytes(http_envelope(locate_body(
                CHANNEL_ID,
                "srvfix000000000000000000ff",
            )))),
            10,
        );
        LocateNoFrameTrace {
            initial,
            effect_count: replay.effects().len(),
            retry: None,
        }
    }
}

/// 逐字段验证 locate Frame 的 envelope、scope、target、anchor、items 与 directive。
pub fn assert_located_contract(frame: &InteractionFrameV2) {
    frame.validate().expect("G11 Frame remains valid");
    assert_eq!(frame.schema_version, 2);
    assert_eq!(frame.causation_id.as_deref(), Some(REQUEST_ID));
    assert_eq!(frame.mutations.len(), 1);
    let mutation = &frame.mutations[0];
    assert_eq!(mutation.mutation.surface, SurfaceId::MessageTimeline);
    assert_eq!(
        mutation.mutation.scope,
        SurfaceScope::Timeline {
            channel_id: CHANNEL_ID.to_string(),
            window_token: WINDOW_TOKEN.to_string(),
        }
    );
    assert_eq!(mutation.mutation.base_revision, Some(1));
    assert_eq!(mutation.mutation.revision, 2);
    assert_eq!(mutation.surface_epoch, 1);
    let TypedSurfaceMutation::TimelinePatch { patch } = &mutation.mutation.mutation else {
        panic!("G11 terminal mutation must be a complete timeline patch");
    };
    assert_eq!(patch.view.channel_id, CHANNEL_ID);
    assert_eq!(patch.view.target_message_id.as_deref(), Some(TARGET_ID));
    assert_eq!(patch.view.anchor.message_id.as_deref(), Some(TARGET_ID));
    assert_eq!(patch.view.anchor.mode, TimelineAnchorMode::Locate);
    assert_eq!(patch.view.items.len(), 20);
    assert!(patch.view.items.iter().any(|item| {
        item.id == TARGET_ID
            && item.content
                == helix_im::interaction_frame::MessageContentVm::Text {
                    text: TARGET_MARKER.to_string(),
                }
    }));
    assert_eq!(frame.directives.len(), 1);
    assert_eq!(frame.directives[0].kind, "revealMessage");
    assert_eq!(
        frame.directives[0].directive_id,
        format!("reveal-{REQUEST_ID}")
    );
}

/// 从共享生产建群回放生成 G11 fresh-group Frame。
fn group_created_frame() -> InteractionFrameV2 {
    let spec = g11_scenario_spec();
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
    /// 创建带 Session store、生产身份与 G11 channel 注册的回放器。
    fn new() -> Self {
        let mut module = ImModule::new_with_local_store(
            ImConfig {
                api_base_url: "http://g11-wdio.invalid".to_string(),
                default_api_base_url: "http://g11-wdio.invalid".to_string(),
                auth_user_id: ACTOR_ID.to_string(),
                company_id: TEAM_ID.to_string(),
                user_name: "G11 Actor".to_string(),
                ..ImConfig::default()
            },
            LocalStoreMode::Session,
        );
        module.register_channel(
            ChannelId::from_str(CHANNEL_ID).expect("G11 channel id fixture is valid"),
            0,
        );
        Self {
            module,
            sink: EffectSink::new(),
            now_ms: CREATED_AT as u64,
        }
    }

    /// 经 G10 local-first production pipeline 建立包含目标的完整 bounded timeline。
    fn attach_timeline(&mut self) -> InteractionFrameV2 {
        self.step(Tick::Command(AppCommand::new(
            "im_query_messages_by_channel",
            serde_json::to_vec(&json!({
                "channel_id": CHANNEL_ID,
                "limit": 20,
                "windowToken": WINDOW_TOKEN,
            }))
            .expect("G11 prerequisite command serializes"),
        )));
        let scan = self.only_persist_corr_with(|ops| {
            ops.iter()
                .any(|op| matches!(op, StorageOp::Scan(spec) if spec.table == "message"))
        });
        self.reply(
            scan,
            PortOutcome::Ok(ReplyBytes(Bytes::from_static(b"[]"))),
            1,
        );
        let http = self.only_http_corr("/posts/getLatestPost");
        self.reply(
            http,
            PortOutcome::Ok(ReplyBytes(http_envelope(latest_posts_body()))),
            2,
        );
        let cache = self.only_persist_corr_with(|ops| {
            ops.iter()
                .any(|op| matches!(op, StorageOp::BatchUpsert(spec) if spec.table == "message"))
        });
        self.reply(cache, PortOutcome::Ok(ReplyBytes::default()), 3);
        self.only_frame()
    }

    /// 提交 canonical locate command 并返回 `/posts/get` correlation。
    fn start_locate(&mut self, post_id: &str) -> Correlation {
        self.step(Tick::Command(AppCommand::new(
            "im_get_posts",
            serde_json::to_vec(&json!({
                "post_ids": [post_id],
                "req_id": REQUEST_ID,
                "locate": true,
            }))
            .expect("G11 locate command serializes"),
        )));
        self.only_http_corr("/posts/get")
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
            .expect("G11 production handle succeeds");
    }

    /// 返回当前 Effect batch。
    fn effects(&self) -> &[Effect] {
        self.sink.as_slice()
    }

    /// 提取满足谓词的唯一 Persist correlation。
    fn only_persist_corr_with(&self, predicate: impl Fn(&[StorageOp]) -> bool) -> Correlation {
        let corrs = self
            .effects()
            .iter()
            .filter_map(|effect| match effect {
                Effect::Persist { corr, ops } if predicate(ops) => Some(*corr),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(corrs.len(), 1, "expected exactly one matching Persist");
        corrs[0]
    }

    /// 提取当前唯一匹配路径的 HTTP correlation。
    fn only_http_corr(&self, path: &str) -> Correlation {
        let corrs = self
            .effects()
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
        self.effects()
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
        assert_eq!(frames.len(), 1, "expected exactly one G11 Frame");
        frames.into_iter().next().expect("one frame exists")
    }

    /// 解码当前 batch 的全部 production InteractionFrameV2。
    fn frames(&self) -> Vec<InteractionFrameV2> {
        self.effects()
            .iter()
            .filter_map(|effect| match effect {
                Effect::Emit { event } => {
                    let envelope: Value =
                        serde_json::from_slice(event.0.as_ref()).expect("G11 event envelope");
                    (envelope["event"] == "im:interaction:frame").then(|| {
                        InteractionFrameV2::decode(
                            &serde_json::to_vec(&envelope["data"]).expect("G11 frame data"),
                        )
                        .expect("G11 emitted Frame decodes")
                    })
                }
                _ => None,
            })
            .collect()
    }
}

/// 构造服务端 recent window，target 固定在第 11 条。
fn latest_posts_body() -> Bytes {
    let posts = (1..=20).rev().map(remote_post).collect::<Vec<_>>();
    Bytes::from(
        serde_json::to_vec(&json!({"status": "SUCCESS", "data": posts}))
            .expect("G11 latest posts body serializes"),
    )
}

/// 构造一个 viewer-visible server Post。
fn remote_post(index: u32) -> Value {
    let id = server_id(index);
    json!({
        "id": id,
        "temporaryId": format!("g11-temporary-{index}"),
        "channelId": CHANNEL_ID,
        "userId": if index % 2 == 0 { ACTOR_ID } else { OBSERVER_ID },
        "type": "TEXT",
        "message": if id == TARGET_ID { TARGET_MARKER.to_string() } else { format!("g11-row-{index}") },
        "simpleMessage": format!("g11-row-{index}"),
        "viewers": ["all"],
        "mentions": [],
        "eventSeq": index,
        "createAt": CREATED_AT + i64::from(index),
    })
}

/// 构造 `/posts/get` 的目标 authority body。
fn locate_body(channel_id: &str, post_id: &str) -> Bytes {
    Bytes::from(
        serde_json::to_vec(&json!({"data": [{
            "id": post_id,
            "channelId": channel_id,
            "userId": OBSERVER_ID,
            "type": "TEXT",
            "message": TARGET_MARKER,
            "createAt": CREATED_AT + 11,
        }]}))
        .expect("G11 locate body serializes"),
    )
}

/// 生成稳定 26 字符 server id。
fn server_id(index: u32) -> String {
    format!("srvfix{index:020x}")
}

/// 将裸 Go body 包成 host 实际回灌的 ADR-007 HTTP envelope。
fn http_envelope(body: Bytes) -> Bytes {
    Bytes::from(
        serde_json::to_vec(&json!({
            "status": 200,
            "headers": {},
            "body": base64_std(body.as_ref()),
        }))
        .expect("G11 HTTP envelope serializes"),
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
