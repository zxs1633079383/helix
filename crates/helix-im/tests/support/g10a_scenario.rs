use bytes::Bytes;
use helix_core::effect::{Correlation, Effect, StorageOp};
use helix_core::tick::{AppCommand, PortError, PortOutcome, ReplyBytes};
use helix_core::{EffectSink, Module, Tick};
use helix_im::interaction_frame::{
    InteractionFrameV2, MessageContentVm, SurfaceId, SurfaceLoadState, SurfaceScope,
    TypedSurfaceMutation,
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

pub const CHANNEL_ID: &str = "chfixx0000000000000000001a";
pub const WINDOW_TOKEN: &str = "g10a-latest-window";
pub const ACTOR_ID: &str = "g10a-wdio-actor";
pub const OBSERVER_ID: &str = "g10a-wdio-observer";
pub const TEAM_ID: &str = "g10a-wdio-company";
pub const CREATED_AT: i64 = 1_700_001_100_000;
pub const GROUP_NAME: &str = "G10a Fixture Group";

/// Returns deterministic identities shared by the G10a group prerequisite and timeline scope.
pub fn g10a_scenario_spec() -> SendScenarioSpec {
    SendScenarioSpec {
        channel_id: CHANNEL_ID,
        server_id: "srvfix00000000000000000028",
        request_id: "g10a-load-request-1",
        marker: "g10a-recent-40",
        simple_message: "g10a-recent-preview",
        message_type: "TEXT",
        props: json!({}),
        actor_id: ACTOR_ID,
        observer_id: OBSERVER_ID,
        team_id: TEAM_ID,
        created_at: CREATED_AT,
        group_name: GROUP_NAME,
        api_base_url: "http://g10a-wdio.invalid",
    }
}

/// Captures the success path around the durable message barrier and final Frame patch.
pub struct LoadOlderSuccessTrace {
    pub group_created: InteractionFrameV2,
    pub initial: InteractionFrameV2,
    pub loaded: InteractionFrameV2,
    pub persisted_before_frame: bool,
}

/// Captures one failed load while retaining the preceding authoritative timeline.
pub struct LoadOlderFailureTrace {
    pub group_created: InteractionFrameV2,
    pub initial: InteractionFrameV2,
    pub failed: InteractionFrameV2,
    pub persisted_write_count: usize,
}

/// Captures duplicate and late reply behavior after the corresponding correlation was consumed.
pub struct LoadOlderRaceTrace {
    pub loaded: InteractionFrameV2,
    pub duplicate_persist_effects: usize,
    pub duplicate_readback_effects: usize,
}

/// Captures a second load that proves the server has no history before the oldest row.
pub struct LoadOlderRecoveryTrace {
    pub loaded: InteractionFrameV2,
    pub exhausted: InteractionFrameV2,
}

/// Drives G10a exclusively through the public production `ImModule::handle` boundary.
pub struct G10AScenarioRunner;

impl G10AScenarioRunner {
    /// Replays fresh group, G10 attachment, G10a HTTP, message persist and durable readback.
    pub fn run_success() -> LoadOlderSuccessTrace {
        let group_created = group_created_frame();
        let mut replay = Replay::new();
        let initial = replay.attach_initial_window();
        let http_corr = replay.start_load(server_id(21).as_str());
        replay.reply(
            http_corr,
            PortOutcome::Ok(ReplyBytes(http_envelope(post_context_body()))),
            10,
        );
        let persisted_before_frame =
            replay.frames().is_empty() && replay.has_message_cache_persist();
        let persist_corr = replay.only_message_write_corr();
        replay.reply(persist_corr, PortOutcome::Ok(ReplyBytes::default()), 11);
        let readback_corr = replay.only_scan_corr();
        replay.reply(
            readback_corr,
            PortOutcome::Ok(ReplyBytes(all_local_rows_bytes())),
            12,
        );
        LoadOlderSuccessTrace {
            group_created,
            initial,
            loaded: replay.only_frame(),
            persisted_before_frame,
        }
    }

    /// Replays transport failure and proves the existing items survive in a failed patch.
    pub fn run_http_failure() -> LoadOlderFailureTrace {
        let group_created = group_created_frame();
        let mut replay = Replay::new();
        let initial = replay.attach_initial_window();
        let http_corr = replay.start_load(server_id(21).as_str());
        replay.reply(http_corr, PortOutcome::Err(PortError::Http(500)), 10);
        LoadOlderFailureTrace {
            group_created,
            initial,
            failed: replay.only_frame(),
            persisted_write_count: replay.persist_write_count(),
        }
    }

    /// Replays storage failure and proves no ready Frame escapes before durable acknowledgement.
    pub fn run_persist_failure() -> LoadOlderFailureTrace {
        let group_created = group_created_frame();
        let mut replay = Replay::new();
        let initial = replay.attach_initial_window();
        let http_corr = replay.start_load(server_id(21).as_str());
        replay.reply(
            http_corr,
            PortOutcome::Ok(ReplyBytes(http_envelope(post_context_body()))),
            10,
        );
        let persist_corr = replay.only_message_write_corr();
        replay.reply(persist_corr, PortOutcome::Err(PortError::Storage(500)), 11);
        LoadOlderFailureTrace {
            group_created,
            initial,
            failed: replay.only_frame(),
            persisted_write_count: 1,
        }
    }

    /// Replays malformed authority data and proves it becomes a failed Frame without writes.
    pub fn run_malformed_response() -> LoadOlderFailureTrace {
        let group_created = group_created_frame();
        let mut replay = Replay::new();
        let initial = replay.attach_initial_window();
        let http_corr = replay.start_load(server_id(21).as_str());
        replay.reply(
            http_corr,
            PortOutcome::Ok(ReplyBytes(http_envelope(Bytes::from_static(b"[]")))),
            10,
        );
        LoadOlderFailureTrace {
            group_created,
            initial,
            failed: replay.only_frame(),
            persisted_write_count: replay.persist_write_count(),
        }
    }

    /// Replays consumed correlations and proves duplicate persist/readback replies are no-ops.
    pub fn run_duplicate_replies() -> LoadOlderRaceTrace {
        let mut replay = Replay::new();
        replay.attach_initial_window();
        let http_corr = replay.start_load(server_id(21).as_str());
        replay.reply(
            http_corr,
            PortOutcome::Ok(ReplyBytes(http_envelope(post_context_body()))),
            10,
        );
        let persist_corr = replay.only_message_write_corr();
        replay.reply(persist_corr, PortOutcome::Ok(ReplyBytes::default()), 11);
        let readback_corr = replay.only_scan_corr();
        replay.reply(persist_corr, PortOutcome::Ok(ReplyBytes::default()), 12);
        let duplicate_persist_effects = replay.effects().len();
        replay.reply(
            readback_corr,
            PortOutcome::Ok(ReplyBytes(all_local_rows_bytes())),
            13,
        );
        let loaded = replay.only_frame();
        replay.reply(
            readback_corr,
            PortOutcome::Ok(ReplyBytes(all_local_rows_bytes())),
            14,
        );
        LoadOlderRaceTrace {
            loaded,
            duplicate_persist_effects,
            duplicate_readback_effects: replay.effects().len(),
        }
    }

    /// Replays a second page whose pivot-only response proves that no older rows remain.
    pub fn run_exhaustion_recovery() -> LoadOlderRecoveryTrace {
        let mut replay = Replay::new();
        replay.attach_initial_window();
        let first_http = replay.start_load(server_id(21).as_str());
        replay.reply(
            first_http,
            PortOutcome::Ok(ReplyBytes(http_envelope(post_context_body()))),
            10,
        );
        let persist_corr = replay.only_message_write_corr();
        replay.reply(persist_corr, PortOutcome::Ok(ReplyBytes::default()), 11);
        let readback_corr = replay.only_scan_corr();
        replay.reply(
            readback_corr,
            PortOutcome::Ok(ReplyBytes(all_local_rows_bytes())),
            12,
        );
        let loaded = replay.only_frame();

        let second_http = replay.start_load(server_id(1).as_str());
        replay.reply(
            second_http,
            PortOutcome::Ok(ReplyBytes(http_envelope(Bytes::from(
                serde_json::to_vec(&vec![remote_post(1, "g10a-older")])
                    .expect("G10a exhausted body serializes"),
            )))),
            20,
        );
        let exhausted_readback = replay.only_scan_corr();
        replay.reply(
            exhausted_readback,
            PortOutcome::Ok(ReplyBytes(all_local_rows_bytes())),
            21,
        );
        LoadOlderRecoveryTrace {
            loaded,
            exhausted: replay.only_frame(),
        }
    }
}

/// Verifies the complete G10a timeline envelope and the Helix-owned page semantics.
pub fn assert_timeline_contract(
    frame: &InteractionFrameV2,
    expected_state: SurfaceLoadState,
    expected_items: usize,
    expected_has_older: bool,
) {
    frame.validate().expect("G10a Frame remains valid");
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
    let view = match &mutation.mutation.mutation {
        TypedSurfaceMutation::TimelineSnapshot { view } => view,
        TypedSurfaceMutation::TimelinePatch { patch } => &patch.view,
        other => panic!("expected G10a timeline mutation, got {other:?}"),
    };
    assert_eq!(view.state, expected_state);
    assert_eq!(view.items.len(), expected_items);
    assert_eq!(view.page.window_token, WINDOW_TOKEN);
    assert_eq!(view.page.has_older, expected_has_older);
    assert_eq!(view.page.has_more, expected_has_older);
    assert!(!view.page.has_newer);
    if expected_items == 40 {
        assert_eq!(
            view.items.first().map(|item| &item.content),
            Some(&MessageContentVm::Text {
                text: "g10a-older-1".to_string(),
            })
        );
        assert_eq!(
            view.items.last().map(|item| &item.content),
            Some(&MessageContentVm::Text {
                text: "g10a-recent-40".to_string(),
            })
        );
    }
}

/// Generates the shared fresh-group Frame without creating a second group reducer.
fn group_created_frame() -> InteractionFrameV2 {
    let spec = g10a_scenario_spec();
    let mut runner = shared::G01ScenarioRunner::new_for(spec.clone());
    let frame = runner.create_group_prerequisite();
    assert_group_created_contract_for(&frame, &spec);
    frame
}

/// Holds one production module and exposes only deterministic Tick/Effect operations.
struct Replay {
    module: ImModule,
    sink: EffectSink,
    now_ms: u64,
}

impl Replay {
    /// Creates a session-store module with the canonical channel registered.
    fn new() -> Self {
        let mut module = ImModule::new_with_local_store(
            ImConfig {
                api_base_url: "http://g10a-wdio.invalid".to_string(),
                default_api_base_url: "http://g10a-wdio.invalid".to_string(),
                auth_user_id: ACTOR_ID.to_string(),
                company_id: TEAM_ID.to_string(),
                user_name: "G10a Actor".to_string(),
                ..ImConfig::default()
            },
            LocalStoreMode::Session,
        );
        module.register_channel(
            ChannelId::from_str(CHANNEL_ID).expect("G10a channel id is valid"),
            0,
        );
        Self {
            module,
            sink: EffectSink::new(),
            now_ms: CREATED_AT as u64,
        }
    }

    /// Establishes the required G10 typed window on the same module before loading older rows.
    fn attach_initial_window(&mut self) -> InteractionFrameV2 {
        self.step(Tick::Command(AppCommand::new(
            "im_query_messages_by_channel",
            serde_json::to_vec(&json!({
                "channel_id": CHANNEL_ID,
                "limit": 20,
                "windowToken": WINDOW_TOKEN,
            }))
            .expect("G10a prerequisite query serializes"),
        )));
        let scan_corr = self.only_scan_corr();
        self.reply(
            scan_corr,
            PortOutcome::Ok(ReplyBytes(Bytes::from_static(b"[]"))),
            1,
        );
        let http_corr = self.only_http_corr("/posts/getLatestPost");
        self.reply(
            http_corr,
            PortOutcome::Ok(ReplyBytes(http_envelope(latest_posts_body()))),
            2,
        );
        let persist_corr = self.only_message_write_corr();
        self.reply(persist_corr, PortOutcome::Ok(ReplyBytes::default()), 3);
        self.only_frame()
    }

    /// Admits the canonical G10a command and returns its first postContext correlation.
    fn start_load(&mut self, anchor_post_id: &str) -> Correlation {
        self.step(Tick::Command(AppCommand::new(
            "im_load_older_context",
            serde_json::to_vec(&json!({
                "channel_id": CHANNEL_ID,
                "anchor_post_id": anchor_post_id,
                "before": 20,
            }))
            .expect("G10a command serializes"),
        )));
        self.only_http_corr("/posts/postContext")
    }

    /// Feeds one deterministic PortReply into the public module boundary.
    fn reply(&mut self, corr: Correlation, outcome: PortOutcome, offset_ms: u64) {
        self.now_ms = (CREATED_AT as u64) + offset_ms;
        self.step(Tick::PortReply { corr, outcome });
    }

    /// Executes one Tick and retains only the Effects produced by that Tick.
    fn step(&mut self, tick: Tick) {
        self.sink.clear();
        self.module
            .handle(&tick, self.now_ms, &mut self.sink)
            .expect("G10a production handle succeeds");
    }

    /// Returns the current deterministic Effect batch.
    fn effects(&self) -> &[Effect] {
        self.sink.as_slice()
    }

    /// Returns the unique storage Scan correlation in the current batch.
    fn only_scan_corr(&self) -> Correlation {
        let corrs = self
            .effects()
            .iter()
            .filter_map(|effect| match effect {
                Effect::Persist { corr, ops }
                    if ops.iter().any(|op| matches!(op, StorageOp::Scan(_))) =>
                {
                    Some(*corr)
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(corrs.len(), 1, "expected one G10a storage scan");
        corrs[0]
    }

    /// Returns the unique message BatchUpsert persistence correlation.
    fn only_message_write_corr(&self) -> Correlation {
        let corrs = self
            .effects()
            .iter()
            .filter_map(|effect| match effect {
                Effect::Persist { corr, ops }
                    if ops.iter().any(
                        |op| matches!(op, StorageOp::BatchUpsert(spec) if spec.table == "message"),
                    ) =>
                {
                    Some(*corr)
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(corrs.len(), 1, "expected one G10a message write");
        corrs[0]
    }

    /// Returns the unique HTTP correlation whose URL has the expected suffix.
    fn only_http_corr(&self, suffix: &str) -> Correlation {
        let corrs = self
            .effects()
            .iter()
            .filter_map(|effect| match effect {
                Effect::Http { corr, req } if req.url.ends_with(suffix) => Some(*corr),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(corrs.len(), 1, "expected one G10a HTTP effect");
        corrs[0]
    }

    /// Decodes the unique InteractionFrame V2 in the current batch.
    fn only_frame(&self) -> InteractionFrameV2 {
        let frames = self.frames();
        assert_eq!(frames.len(), 1, "expected one G10a Frame");
        frames.into_iter().next().expect("one G10a Frame exists")
    }

    /// Decodes every InteractionFrame V2 in the current batch.
    fn frames(&self) -> Vec<InteractionFrameV2> {
        self.effects()
            .iter()
            .filter_map(|effect| match effect {
                Effect::Emit { event } => {
                    let envelope: Value =
                        serde_json::from_slice(event.0.as_ref()).expect("G10a event envelope");
                    (envelope["event"] == "im:interaction:frame").then(|| {
                        InteractionFrameV2::decode(
                            &serde_json::to_vec(&envelope["data"]).expect("G10a frame data"),
                        )
                        .expect("G10a Frame decodes")
                    })
                }
                _ => None,
            })
            .collect()
    }

    /// Reports whether the current batch contains a durable message BatchUpsert.
    fn has_message_cache_persist(&self) -> bool {
        self.effects().iter().any(|effect| match effect {
            Effect::Persist { ops, .. } => ops
                .iter()
                .any(|op| matches!(op, StorageOp::BatchUpsert(spec) if spec.table == "message")),
            _ => false,
        })
    }

    /// Counts persistence writes while excluding storage Scan operations.
    fn persist_write_count(&self) -> usize {
        self.effects()
            .iter()
            .filter(|effect| match effect {
                Effect::Persist { ops, .. } => {
                    ops.iter().any(|op| !matches!(op, StorageOp::Scan(_)))
                }
                Effect::PersistAtomic { .. } | Effect::PersistFire { .. } => true,
                _ => false,
            })
            .count()
    }
}

/// Builds the latest 20 authoritative posts returned in server-descending order.
fn latest_posts_body() -> Bytes {
    let posts = (21..=40)
        .rev()
        .map(|index| remote_post(index, "g10a-recent"))
        .collect::<Vec<_>>();
    Bytes::from(
        serde_json::to_vec(&json!({"status": "SUCCESS", "data": posts}))
            .expect("G10a latest body serializes"),
    )
}

/// Builds one postContext page containing the anchor and 20 strictly older posts.
fn post_context_body() -> Bytes {
    let mut posts = vec![remote_post(21, "g10a-recent")];
    posts.extend((1..=20).rev().map(|index| remote_post(index, "g10a-older")));
    Bytes::from(serde_json::to_vec(&posts).expect("G10a context body serializes"))
}

/// Builds the complete durable readback in storage-descending order.
fn all_local_rows_bytes() -> Bytes {
    let rows = (1..=40)
        .rev()
        .map(|index| {
            let marker = if index <= 20 {
                "g10a-older"
            } else {
                "g10a-recent"
            };
            json!({
                "id": server_id(index),
                "temporary_id": format!("g10a-temporary-{index}"),
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
    Bytes::from(serde_json::to_vec(&rows).expect("G10a readback serializes"))
}

/// Builds one viewer-visible Go wire Post.
fn remote_post(index: u32, marker: &str) -> Value {
    json!({
        "id": server_id(index),
        "temporaryId": format!("g10a-temporary-{index}"),
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

/// Returns a stable 26-character server id.
fn server_id(index: u32) -> String {
    format!("srvfix{index:020x}")
}

/// Wraps a raw Go body in the production ADR-007 host HTTP envelope.
fn http_envelope(body: Bytes) -> Bytes {
    Bytes::from(
        serde_json::to_vec(&json!({
            "status": 200,
            "headers": {},
            "body": base64_std(body.as_ref()),
        }))
        .expect("G10a HTTP envelope serializes"),
    )
}

/// Encodes bytes with the deterministic standard Base64 alphabet used by host replies.
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
