use bytes::Bytes;
use helix_core::effect::{Correlation, Effect, StorageOp, TimerId};
use helix_core::tick::{AppCommand, InboundBytes, PortError, PortOutcome, ReplyBytes};
use helix_core::{EffectSink, Module, Tick};
use helix_im::module::ImConfig;
use helix_im::ImModule;
use serde_json::{json, Value};

pub const CHANNEL_ID: &str = "chflow00000000000000000001";
pub const SERVER_ID: &str = "svflow00000000000000000001";
pub const ACTOR_ID: &str = "g01-flow-actor";
pub const TEAM_ID: &str = "g01-flow-team";
pub const REQUEST_ID: &str = "g01-flow-request";
pub const TEXT: &str = "g01 complete flow";
pub const CREATED_AT: i64 = 1_700_000_100_000;

pub struct Replay {
    pub module: ImModule,
    pub sink: EffectSink,
}

impl Replay {
    pub fn new() -> Self {
        Self {
            module: ImModule::new(ImConfig {
                api_base_url: "http://g01-flow.invalid".to_string(),
                default_api_base_url: "http://g01-flow.invalid".to_string(),
                auth_user_id: ACTOR_ID.to_string(),
                company_id: TEAM_ID.to_string(),
                user_name: "G01 Flow Actor".to_string(),
                ..ImConfig::default()
            }),
            sink: EffectSink::new(),
        }
    }

    pub fn step(&mut self, tick: Tick, now_ms: u64) {
        self.sink.clear();
        self.module
            .handle(&tick, now_ms, &mut self.sink)
            .expect("G01 replay step must succeed");
    }

    pub fn effects(&self) -> &[Effect] {
        self.sink.as_slice()
    }

    pub fn only_http_corr(&self, suffix: &str) -> Correlation {
        let mut matches = self.effects().iter().filter_map(|effect| match effect {
            Effect::Http { corr, req } if req.url.ends_with(suffix) => Some(*corr),
            _ => None,
        });
        let corr = matches.next().expect("expected one matching HTTP effect");
        assert!(
            matches.next().is_none(),
            "HTTP correlation must be unambiguous for {suffix}"
        );
        corr
    }

    pub fn only_persist_corr_with(&self, predicate: impl Fn(&[StorageOp]) -> bool) -> Correlation {
        let mut matches = self.effects().iter().filter_map(|effect| match effect {
            Effect::Persist { corr, ops } if predicate(ops) => Some(*corr),
            _ => None,
        });
        let corr = matches
            .next()
            .expect("expected one matching Persist effect");
        assert!(
            matches.next().is_none(),
            "Persist correlation must be causally unambiguous"
        );
        corr
    }

    pub fn event(&self, name: &str) -> Option<Value> {
        self.effects().iter().find_map(|effect| {
            let Effect::Emit { event } = effect else {
                return None;
            };
            let envelope: Value = serde_json::from_slice(event.0.as_ref()).ok()?;
            (envelope["event"] == name).then_some(envelope)
        })
    }

    pub fn frames(&self) -> Vec<Value> {
        self.effects()
            .iter()
            .filter_map(|effect| {
                let Effect::Emit { event } = effect else {
                    return None;
                };
                let envelope: Value = serde_json::from_slice(event.0.as_ref()).ok()?;
                (envelope["event"] == "im:interaction:frame")
                    .then(|| envelope.get("data").cloned())
                    .flatten()
            })
            .collect()
    }
}

pub struct SendingStage {
    pub temporary_id: String,
    pub http_corr: Correlation,
    pub timer_id: TimerId,
}

pub fn command(name: &'static str, payload: Value) -> Tick {
    Tick::Command(AppCommand::new(
        name,
        serde_json::to_vec(&payload).expect("command payload serializes"),
    ))
}

pub fn ok(corr: Correlation, bytes: Bytes) -> Tick {
    Tick::PortReply {
        corr,
        outcome: PortOutcome::Ok(ReplyBytes(bytes)),
    }
}

pub fn http_error(corr: Correlation) -> Tick {
    Tick::PortReply {
        corr,
        outcome: PortOutcome::Err(PortError::Http(500)),
    }
}

pub fn http_envelope(body: Value) -> Bytes {
    let raw = serde_json::to_vec(&body).expect("HTTP body serializes");
    Bytes::from(
        serde_json::to_vec(&json!({
            "status": 200,
            "headers": {},
            "body": base64(&raw),
        }))
        .expect("HTTP envelope serializes"),
    )
}

pub fn attach_dialog(replay: &mut Replay) {
    replay.step(
        command("im_query_dialog_list", json!({})),
        CREATED_AT as u64,
    );
    let corr = replay.only_persist_corr_with(|ops| {
        ops.iter()
            .any(|op| matches!(op, StorageOp::Scan(spec) if spec.table == "channel"))
    });
    replay.step(
        ok(
            corr,
            Bytes::from(
                serde_json::to_vec(&json!([channel_row(0, "", CREATED_AT - 1)]))
                    .expect("dialog row serializes"),
            ),
        ),
        CREATED_AT as u64 + 1,
    );
    assert_frame_surface(replay, "message.left-dialog-list");
}

pub fn attach_timeline(replay: &mut Replay) {
    replay.step(
        command(
            "im_query_messages_by_channel",
            json!({
                "channel_id": CHANNEL_ID,
                "limit": 20,
                "windowToken": "latest",
            }),
        ),
        CREATED_AT as u64 + 2,
    );
    let local_corr = replay.only_persist_corr_with(|ops| {
        ops.iter()
            .any(|op| matches!(op, StorageOp::Scan(spec) if spec.table == "message"))
    });
    replay.step(
        ok(local_corr, Bytes::from_static(b"[]")),
        CREATED_AT as u64 + 3,
    );
    let remote_corr = replay.only_http_corr("/posts/getLatestPost");
    replay.step(
        ok(
            remote_corr,
            http_envelope(json!({"status": "SUCCESS", "data": []})),
        ),
        CREATED_AT as u64 + 4,
    );
    assert_frame_surface(replay, "message.timeline");

    replay.step(
        command(
            "__helix_request_surface_snapshot",
            json!({
                "surface": "message.timeline",
                "scope": {
                    "kind": "timeline",
                    "channelId": CHANNEL_ID,
                    "windowToken": "latest"
                },
                "causationId": "g01-flow-attach"
            }),
        ),
        CREATED_AT as u64 + 5,
    );
    let _attachment_read = replay.only_persist_corr_with(|ops| {
        ops.iter()
            .any(|op| matches!(op, StorageOp::Scan(spec) if spec.table == "message"))
    });
}

pub fn start_sending(replay: &mut Replay) -> SendingStage {
    replay.step(
        command(
            "im_send_message",
            json!({
                "channel_id": CHANNEL_ID,
                "text": TEXT,
                "type": "TEXT",
                "req_id": REQUEST_ID,
            }),
        ),
        CREATED_AT as u64 + 10,
    );
    assert!(
        replay
            .effects()
            .iter()
            .all(|effect| !matches!(effect, Effect::Http { .. })),
        "posts/create must not run before the optimistic Persist barrier"
    );
    assert!(
        replay.frames().is_empty(),
        "a render-ready timeline Frame requires durable readback"
    );
    let temporary_id = replay.event("im:post:sending").expect("sending projection")["data"]
        ["temporaryId"]
        .as_str()
        .expect("temporaryId")
        .to_string();
    let p1_corr = replay.only_persist_corr_with(|ops| has_table_write(ops, "message"));

    replay.step(ok(p1_corr, Bytes::new()), CREATED_AT as u64 + 11);
    assert!(
        replay
            .effects()
            .iter()
            .all(|effect| !matches!(effect, Effect::Http { .. })),
        "attached timeline must publish sending before posts/create starts"
    );
    let read_corr = replay.only_persist_corr_with(|ops| {
        ops.iter()
            .any(|op| matches!(op, StorageOp::Scan(spec) if spec.table == "message"))
    });
    replay.step(
        ok(
            read_corr,
            Bytes::from(
                serde_json::to_vec(&json!([message_row(
                    &temporary_id,
                    &temporary_id,
                    "sending"
                )]))
                .expect("sending row serializes"),
            ),
        ),
        CREATED_AT as u64 + 12,
    );
    assert_delivery(replay, "sending", &temporary_id);
    let http_corr = replay.only_http_corr("/posts/create");
    let timer_id = replay
        .effects()
        .iter()
        .find_map(|effect| match effect {
            Effect::ScheduleTimer { id, after_ms } if *after_ms == 15_000 => Some(*id),
            _ => None,
        })
        .expect("sending stage schedules the 15 second timeout");

    SendingStage {
        temporary_id,
        http_corr,
        timer_id,
    }
}

pub fn message_row(id: &str, temporary_id: &str, status: &str) -> Value {
    json!({
        "id": id,
        "temporaryId": temporary_id,
        "channelId": CHANNEL_ID,
        "userId": ACTOR_ID,
        "createAt": CREATED_AT,
        "message": TEXT,
        "type": "TEXT",
        "sendStatus": status,
        "props": {},
    })
}

pub fn channel_row(unread_count: i64, last_post: &str, last_root_post_at: i64) -> Value {
    json!({
        "id": CHANNEL_ID,
        "team_id": TEAM_ID,
        "display_name": "G01 Flow Group",
        "type": "P",
        "unread_count": unread_count,
        "mention_count": 0,
        "urgent_count": 0,
        "mention_list": [],
        "urgent_post_list": [],
        "unread_post_id": "",
        "last_post": last_post,
        "last_post_at": last_root_post_at,
        "last_root_post_at": last_root_post_at,
    })
}

pub fn echo_frame(temporary_id: &str) -> Tick {
    let payload = json!({
        "v": 1,
        "action": "post",
        "data": {
            "id": SERVER_ID,
            "temporaryId": temporary_id,
            "channelId": CHANNEL_ID,
            "event_seq": 1,
            "createAt": CREATED_AT + 20,
            "message": TEXT,
            "simpleMessage": TEXT,
            "type": "TEXT",
            "userId": ACTOR_ID,
            "userSnapshot": {"userId": ACTOR_ID, "teamId": TEAM_ID},
            "viewers": ["all"],
            "mentions": [],
            "props": {"channel_event_seq": 1}
        },
        "broadcast": {},
        "seq": 1
    });
    Tick::Inbound(InboundBytes(
        serde_json::to_vec(&payload)
            .expect("WS echo serializes")
            .into(),
    ))
}

pub fn has_table_write(ops: &[StorageOp], table: &str) -> bool {
    ops.iter().any(|op| match op {
        StorageOp::BatchUpsert(spec) => spec.table == table,
        StorageOp::BatchUpdate(spec) => spec.table == table,
        StorageOp::GuardedBump(spec) => spec.table == table,
        _ => false,
    })
}

pub fn assert_frame_surface(replay: &Replay, expected: &str) {
    let frames = replay.frames();
    assert!(
        frames.iter().any(|frame| {
            frame["schemaVersion"] == 2
                && frame["mutations"]
                    .as_array()
                    .is_some_and(|mutations| mutations.iter().any(|m| m["surface"] == expected))
        }),
        "missing V2 surface {expected}; frames={frames:?}"
    );
}

pub fn assert_delivery(replay: &Replay, expected: &str, expected_id: &str) {
    let items = replay
        .frames()
        .into_iter()
        .flat_map(|frame| frame["mutations"].as_array().cloned().unwrap_or_default())
        .filter(|mutation| mutation["surface"] == "message.timeline")
        .flat_map(|mutation| {
            mutation["mutation"]["view"]["items"]
                .as_array()
                .or_else(|| mutation["mutation"]["patch"]["view"]["items"].as_array())
                .cloned()
                .unwrap_or_default()
        })
        .collect::<Vec<_>>();
    assert!(
        items
            .iter()
            .any(|item| { item["id"] == expected_id && item["delivery"]["kind"] == expected }),
        "missing timeline item id={expected_id} delivery={expected}; items={items:?}"
    );
}

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
