use bytes::Bytes;
use helix_core::effect::{Correlation, Effect, StorageOp};
use helix_core::tick::{AppCommand, PortError, PortOutcome, ReplyBytes};
use helix_core::{EffectSink, Module, Tick};
use helix_im::interaction_frame::{
    InteractionFrameV2, SurfaceId, SurfaceLoadState, SurfaceScope, TypedSurfaceMutation,
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

pub const CHANNEL_ID: &str = "chfixx00000000000000000010";
pub const WINDOW_TOKEN: &str = "g10-latest-window";
pub const ACTOR_ID: &str = "g10-wdio-actor";
pub const OBSERVER_ID: &str = "g10-wdio-observer";
pub const TEAM_ID: &str = "g10-wdio-company";
pub const GROUP_NAME: &str = "G10 Fixture Group";
pub const CREATED_AT: i64 = 1_700_001_000_000;

/// 返回 G10 fresh-group、身份、时钟与唯一 timeline scope 的确定性参数。
pub fn g10_scenario_spec() -> SendScenarioSpec {
    SendScenarioSpec {
        channel_id: CHANNEL_ID,
        server_id: "srvg1000000000000000000001",
        request_id: "g10-query-request-1",
        marker: "g10-latest-marker-20",
        simple_message: "g10-latest-preview",
        message_type: "TEXT",
        props: json!({}),
        actor_id: ACTOR_ID,
        observer_id: OBSERVER_ID,
        team_id: TEAM_ID,
        created_at: CREATED_AT,
        group_name: GROUP_NAME,
        api_base_url: "http://g10-wdio.invalid",
    }
}

/// 汇总 G10 fresh-group、首屏 snapshot 与本地覆盖恢复 patch。
#[allow(dead_code)]
pub struct TimelineSuccessTrace {
    pub group_created: InteractionFrameV2,
    pub ready: InteractionFrameV2,
    pub recovered: InteractionFrameV2,
    pub remote_cache_persisted_before_ready: bool,
    pub recovery_http_count: usize,
}

/// 汇总 G10 权威失败 Frame，证明空成功列表不能替代失败语义。
#[allow(dead_code)]
pub struct TimelineFailureTrace {
    pub group_created: InteractionFrameV2,
    pub failed: InteractionFrameV2,
    pub persisted_write_count: usize,
}

/// 汇总 G10 cache persist 失败后的只读 Frame 与 coverage 行为。
#[allow(dead_code)]
pub struct TimelinePersistFailureTrace {
    pub group_created: InteractionFrameV2,
    pub ready_after_cache_failure: InteractionFrameV2,
    pub follow_up_http_count: usize,
}

/// 汇总同 scope 新查询取代旧查询后的唯一终态。
#[allow(dead_code)]
pub struct TimelineRaceTrace {
    pub group_created: InteractionFrameV2,
    pub current: InteractionFrameV2,
    pub late_effect_count: usize,
    pub duplicate_effect_count: usize,
}

/// 通过公开 `ImModule::handle` 驱动 G10 local-first 查询与完整 Frame。
pub struct G10ScenarioRunner;

#[allow(dead_code)]
impl G10ScenarioRunner {
    /// 回放 fresh group、远端首屏、cache barrier 与本地 coverage 恢复。
    pub fn run_success_and_recovery() -> TimelineSuccessTrace {
        let group_created = group_created_frame();
        let mut replay = Replay::new();
        let scan_corr = replay.start_query();
        replay.reply(
            scan_corr,
            PortOutcome::Ok(ReplyBytes(Bytes::from_static(b"[]"))),
            1,
        );
        let http_corr = replay.only_http_corr();
        replay.reply(
            http_corr,
            PortOutcome::Ok(ReplyBytes(http_envelope(latest_posts_body(
                "g10-latest-marker",
            )))),
            2,
        );
        let remote_cache_persisted_before_ready =
            replay.frames().is_empty() && replay.has_message_cache_persist();
        let cache_corr = replay.only_persist_corr();
        replay.reply(cache_corr, PortOutcome::Ok(ReplyBytes::default()), 3);
        let ready = replay.only_frame();

        let recovery_scan = replay.start_query();
        replay.reply(
            recovery_scan,
            PortOutcome::Ok(ReplyBytes(local_rows_bytes("g10-latest-marker"))),
            4,
        );
        let recovery_http_count = replay.http_count();
        let recovered = replay.only_frame();
        TimelineSuccessTrace {
            group_created,
            ready,
            recovered,
            remote_cache_persisted_before_ready,
            recovery_http_count,
        }
    }

    /// 回放远端 transport 失败并取得唯一 failed timeline Frame。
    pub fn run_http_failure() -> TimelineFailureTrace {
        let group_created = group_created_frame();
        let mut replay = Replay::new();
        let scan_corr = replay.start_query();
        replay.reply(
            scan_corr,
            PortOutcome::Ok(ReplyBytes(Bytes::from_static(b"[]"))),
            1,
        );
        let http_corr = replay.only_http_corr();
        replay.reply(http_corr, PortOutcome::Err(PortError::Http(500)), 2);
        TimelineFailureTrace {
            group_created,
            failed: replay.only_frame(),
            persisted_write_count: replay.persist_write_count(),
        }
    }

    /// 回放 cache persist 失败，证明 Frame 可展示远端只读事实但不建立 durable coverage。
    pub fn run_cache_persist_failure() -> TimelinePersistFailureTrace {
        let group_created = group_created_frame();
        let mut replay = Replay::new();
        let scan_corr = replay.start_query();
        replay.reply(
            scan_corr,
            PortOutcome::Ok(ReplyBytes(Bytes::from_static(b"[]"))),
            1,
        );
        let http_corr = replay.only_http_corr();
        replay.reply(
            http_corr,
            PortOutcome::Ok(ReplyBytes(http_envelope(latest_posts_body(
                "g10-cache-failure",
            )))),
            2,
        );
        let cache_corr = replay.only_persist_corr();
        replay.reply(cache_corr, PortOutcome::Err(PortError::Storage(500)), 3);
        let ready_after_cache_failure = replay.only_frame();

        let follow_up_scan = replay.start_query();
        replay.reply(
            follow_up_scan,
            PortOutcome::Ok(ReplyBytes(local_rows_bytes("g10-cache-failure"))),
            4,
        );
        TimelinePersistFailureTrace {
            group_created,
            ready_after_cache_failure,
            follow_up_http_count: replay.http_count(),
        }
    }

    /// 回放同 scope 新请求覆盖旧请求，并证明迟到与重复 reply 都不产生第二终态。
    pub fn run_latest_query_wins() -> TimelineRaceTrace {
        let group_created = group_created_frame();
        let mut replay = Replay::new();
        let old_scan = replay.start_query();
        replay.reply(
            old_scan,
            PortOutcome::Ok(ReplyBytes(Bytes::from_static(b"[]"))),
            1,
        );
        let old_http = replay.only_http_corr();

        let current_scan = replay.start_query();
        replay.reply(
            current_scan,
            PortOutcome::Ok(ReplyBytes(Bytes::from_static(b"[]"))),
            2,
        );
        let current_http = replay.only_http_corr();

        replay.reply(
            old_http,
            PortOutcome::Ok(ReplyBytes(http_envelope(latest_posts_body("g10-stale")))),
            3,
        );
        let late_effect_count = replay.effects().len();
        replay.reply(
            current_http,
            PortOutcome::Ok(ReplyBytes(http_envelope(latest_posts_body("g10-current")))),
            4,
        );
        let cache_corr = replay.only_persist_corr();
        replay.reply(cache_corr, PortOutcome::Ok(ReplyBytes::default()), 5);
        let current = replay.only_frame();

        replay.reply(cache_corr, PortOutcome::Ok(ReplyBytes::default()), 6);
        let duplicate_effect_count = replay.effects().len();
        TimelineRaceTrace {
            group_created,
            current,
            late_effect_count,
            duplicate_effect_count,
        }
    }
}

/// 逐字段验证 G10 timeline Frame 的 envelope、scope、revision、page、items 与 action。
pub fn assert_timeline_contract(
    frame: &InteractionFrameV2,
    expected_state: SurfaceLoadState,
    expected_marker: Option<&str>,
    expected_has_older: bool,
) {
    frame.validate().expect("G10 Frame remains valid");
    assert_eq!(frame.schema_version, 2);
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
    assert!(mutation.mutation.revision > 0);
    assert_eq!(mutation.surface_epoch, 1);
    let view = match &mutation.mutation.mutation {
        TypedSurfaceMutation::TimelineSnapshot { view } => view,
        TypedSurfaceMutation::TimelinePatch { patch } => &patch.view,
        other => panic!("expected timeline mutation, got {other:?}"),
    };
    assert_eq!(view.channel_id, CHANNEL_ID);
    assert_eq!(view.page.window_token, WINDOW_TOKEN);
    assert_eq!(view.state, expected_state);
    match expected_marker {
        Some(marker) => {
            assert_eq!(view.items.len(), 20);
            assert_eq!(
                view.items.last().map(|item| item.content.clone()),
                Some(helix_im::interaction_frame::MessageContentVm::Text {
                    text: format!("{marker}-20"),
                })
            );
            assert_eq!(view.page.has_older, expected_has_older);
            assert_eq!(view.page.has_more, expected_has_older);
            assert!(!view.page.has_newer);
            assert!(view
                .items
                .iter()
                .flat_map(|item| &item.actions)
                .any(|action| action.kind == "reply"));
        }
        None => {
            assert!(view.items.is_empty());
            assert!(!view.page.has_older && !view.page.has_newer && !view.page.has_more);
        }
    }
}

/// 从共享生产建群回放生成 G10 fresh-group Frame。
fn group_created_frame() -> InteractionFrameV2 {
    let spec = g10_scenario_spec();
    let mut runner = shared::G01ScenarioRunner::new_for(spec.clone());
    let frame = runner.create_group_prerequisite();
    assert_group_created_contract_for(&frame, &spec);
    frame
}

/// 持有单个生产 ImModule、当前 Effect batch 与确定性时钟。
struct Replay {
    module: ImModule,
    sink: EffectSink,
    now_ms: u64,
}

impl Replay {
    /// 创建带生产身份、Session store 和 G10 channel 注册的回放器。
    fn new() -> Self {
        let mut module = ImModule::new_with_local_store(
            ImConfig {
                api_base_url: "http://g10-wdio.invalid".to_string(),
                default_api_base_url: "http://g10-wdio.invalid".to_string(),
                auth_user_id: ACTOR_ID.to_string(),
                company_id: TEAM_ID.to_string(),
                user_name: "G10 Actor".to_string(),
                ..ImConfig::default()
            },
            LocalStoreMode::Session,
        );
        module.register_channel(
            ChannelId::from_str(CHANNEL_ID).expect("G10 channel id fixture is valid"),
            0,
        );
        Self {
            module,
            sink: EffectSink::new(),
            now_ms: CREATED_AT as u64,
        }
    }

    /// 提交 canonical G10 command 并返回 local scan correlation。
    fn start_query(&mut self) -> Correlation {
        self.step(Tick::Command(AppCommand::new(
            "im_query_messages_by_channel",
            serde_json::to_vec(&json!({
                "channel_id": CHANNEL_ID,
                "limit": 20,
                "windowToken": WINDOW_TOKEN,
            }))
            .expect("G10 command serializes"),
        )));
        self.only_persist_corr()
    }

    /// 回灌一个端口结果并推进确定性时钟。
    fn reply(&mut self, corr: Correlation, outcome: PortOutcome, offset_ms: u64) {
        self.now_ms = (CREATED_AT as u64) + offset_ms;
        self.step(Tick::PortReply { corr, outcome });
    }

    /// 通过公开 Module trait 执行一步并仅保留该步 Effect。
    fn step(&mut self, tick: Tick) {
        self.sink.clear();
        self.module
            .handle(&tick, self.now_ms, &mut self.sink)
            .expect("G10 production handle succeeds");
    }

    /// 返回当前 Effect batch。
    fn effects(&self) -> &[Effect] {
        self.sink.as_slice()
    }

    /// 提取当前唯一 Persist correlation。
    fn only_persist_corr(&self) -> Correlation {
        let corrs = self
            .effects()
            .iter()
            .filter_map(|effect| match effect {
                Effect::Persist { corr, .. } => Some(*corr),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(corrs.len(), 1, "expected exactly one Persist correlation");
        corrs[0]
    }

    /// 提取当前唯一 HTTP correlation。
    fn only_http_corr(&self) -> Correlation {
        let corrs = self
            .effects()
            .iter()
            .filter_map(|effect| match effect {
                Effect::Http { corr, req } if req.url.ends_with("/posts/getLatestPost") => {
                    Some(*corr)
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(corrs.len(), 1, "expected exactly one G10 HTTP correlation");
        corrs[0]
    }

    /// 解码当前 batch 中唯一生产 InteractionFrameV2。
    fn only_frame(&self) -> InteractionFrameV2 {
        let frames = self.frames();
        assert_eq!(frames.len(), 1, "expected exactly one G10 Frame");
        frames.into_iter().next().expect("one frame exists")
    }

    /// 解码当前 batch 的全部生产 InteractionFrameV2。
    fn frames(&self) -> Vec<InteractionFrameV2> {
        self.effects()
            .iter()
            .filter_map(|effect| match effect {
                Effect::Emit { event } => {
                    let envelope: Value =
                        serde_json::from_slice(event.0.as_ref()).expect("G10 event envelope");
                    (envelope["event"] == "im:interaction:frame").then(|| {
                        InteractionFrameV2::decode(
                            &serde_json::to_vec(&envelope["data"]).expect("G10 frame data"),
                        )
                        .expect("G10 emitted Frame decodes")
                    })
                }
                _ => None,
            })
            .collect()
    }

    /// 判断当前 batch 是否含 message cache 写入。
    fn has_message_cache_persist(&self) -> bool {
        self.effects().iter().any(|effect| match effect {
            Effect::Persist { ops, .. } => ops
                .iter()
                .any(|op| matches!(op, StorageOp::BatchUpsert(spec) if spec.table == "message")),
            _ => false,
        })
    }

    /// 统计当前 batch 的 HTTP Effect。
    fn http_count(&self) -> usize {
        self.effects()
            .iter()
            .filter(|effect| matches!(effect, Effect::Http { .. }))
            .count()
    }

    /// 统计当前 batch 的持久写 Effect。
    fn persist_write_count(&self) -> usize {
        self.effects()
            .iter()
            .filter(|effect| {
                matches!(
                    effect,
                    Effect::Persist { .. }
                        | Effect::PersistAtomic { .. }
                        | Effect::PersistFire { .. }
                )
            })
            .count()
    }
}

/// 构造 20 条真 Go recent posts，保持服务端倒序返回。
fn latest_posts_body(marker: &str) -> Bytes {
    let posts = (1..=20)
        .rev()
        .map(|index| remote_post(index, marker))
        .collect::<Vec<_>>();
    Bytes::from(
        serde_json::to_vec(&json!({"status": "SUCCESS", "data": posts}))
            .expect("G10 latest posts body serializes"),
    )
}

/// 构造与 cache upsert 后 SQLite scan 等价的倒序本地 rows。
fn local_rows_bytes(marker: &str) -> Bytes {
    let rows = (1..=20)
        .rev()
        .map(|index| {
            json!({
                "id": server_id(index),
                "temporary_id": format!("g10-temporary-{index}"),
                "channel_id": CHANNEL_ID,
                "user_id": if index % 2 == 0 { ACTOR_ID } else { OBSERVER_ID },
                "type": "TEXT",
                "message": format!("{marker}-{index}"),
                "simple_message": format!("{marker}-{index}"),
                "create_at": CREATED_AT + i64::from(index),
                "send_status": "sent",
            })
        })
        .collect::<Vec<_>>();
    Bytes::from(serde_json::to_vec(&rows).expect("G10 local rows serialize"))
}

/// 构造一条 viewer-visible recent post。
fn remote_post(index: u32, marker: &str) -> Value {
    json!({
        "id": server_id(index),
        "temporaryId": format!("g10-temporary-{index}"),
        "channelId": CHANNEL_ID,
        "userId": if index % 2 == 0 { ACTOR_ID } else { OBSERVER_ID },
        "type": "TEXT",
        "message": format!("{marker}-{index}"),
        "simpleMessage": format!("{marker}-{index}"),
        "viewers": ["all"],
        "mentions": [],
        "eventSeq": index,
        "createAt": CREATED_AT + i64::from(index),
    })
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
        .expect("G10 HTTP envelope serializes"),
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
