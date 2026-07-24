//! G01 WDIO Frame V2 fixture 真源。
//!
//! 这些 fixture 不是测试手写的 ViewModel。测试通过公开 `ImModule::handle` 入口回放
//! `query -> send -> PersistOk -> local readback -> HTTP -> PersistOk -> local readback`，
//! 再保存 Helix 当前 producer/serializer 发出的完整 `InteractionFrameV2`。

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use bytes::Bytes;
use helix_core::effect::{Correlation, Effect};
use helix_core::tick::{AppCommand, PortError, PortOutcome, ReplyBytes};
use helix_core::{EffectSink, Module, Tick};
use helix_im::interaction_frame::{InteractionFrameV2, SurfaceId, TypedSurfaceMutation};
use helix_im::module::ImConfig;
use helix_im::ImModule;
use serde_json::{json, Value};

const CHANNEL_ID: &str = "chfixx00000000000000000001";
const SERVER_ID: &str = "srvfix00000000000000000001";
const REQUEST_ID: &str = "g01-wdio-request-1";
const MARKER: &str = "g01-wdio-fixture-marker";
const CREATED_AT: i64 = 1_700_000_000_002;
const HELIX_COMMIT: &str = "32e3a0659e4f29add1284be2ba1972eb28f0f938";
const GENERATED_AT: &str = "2026-07-24T04:26:11-07:00";
const PRODUCER: &str = concat!(
    "ImModule::handle -> handle_optimistic_send_reply -> ",
    "refresh_attached_latest_timeline_with_deferred_send -> ",
    "emit_timeline_snapshot_with_causation -> projector::emit_interaction_frame"
);

struct Replay {
    module: ImModule,
    sink: EffectSink,
}

impl Replay {
    fn new() -> Self {
        let config = ImConfig {
            api_base_url: "http://g01-wdio.invalid".to_string(),
            default_api_base_url: "http://g01-wdio.invalid".to_string(),
            auth_user_id: "g01-wdio-actor".to_string(),
            company_id: "g01-wdio-company".to_string(),
            user_name: "G01 Actor".to_string(),
            ..ImConfig::default()
        };
        Self {
            module: ImModule::new(config),
            sink: EffectSink::new(),
        }
    }

    fn step(&mut self, tick: Tick, now_ms: u64) {
        self.sink.clear();
        self.module
            .handle(&tick, now_ms, &mut self.sink)
            .expect("G01 public producer replay must succeed");
    }

    fn only_corr(&self, kind: CorrKind) -> Correlation {
        let mut matches = self
            .sink
            .as_slice()
            .iter()
            .filter_map(|effect| match (kind, effect) {
                (CorrKind::Persist, Effect::Persist { corr, .. }) => Some(*corr),
                (CorrKind::Http, Effect::Http { corr, .. }) => Some(*corr),
                _ => None,
            });
        let corr = matches.next().expect("expected one matching correlation");
        assert!(
            matches.next().is_none(),
            "fixture replay must have one unambiguous matching correlation"
        );
        corr
    }

    fn emitted_frame(&self) -> InteractionFrameV2 {
        self.sink
            .as_slice()
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
                    .expect("Helix producer output must decode as InteractionFrameV2")
            })
            .next()
            .expect("expected one emitted InteractionFrameV2")
    }
}

#[derive(Clone, Copy)]
enum CorrKind {
    Persist,
    Http,
}

#[derive(Clone, Copy)]
enum Terminal {
    Sent,
    Failed,
}

fn command(name: &'static str, payload: Value) -> Tick {
    Tick::Command(AppCommand::new(
        name,
        serde_json::to_vec(&payload).expect("command fixture serializes"),
    ))
}

fn ok(corr: Correlation, bytes: Bytes) -> Tick {
    Tick::PortReply {
        corr,
        outcome: PortOutcome::Ok(ReplyBytes(bytes)),
    }
}

fn http_error(corr: Correlation) -> Tick {
    Tick::PortReply {
        corr,
        outcome: PortOutcome::Err(PortError::Http(500)),
    }
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

fn http_envelope(body: Value) -> Bytes {
    let raw = serde_json::to_vec(&body).expect("HTTP fixture serializes");
    Bytes::from(
        serde_json::to_vec(&json!({
            "status": 200,
            "headers": {},
            "body": base64(&raw),
        }))
        .expect("HTTP envelope serializes"),
    )
}

fn attach_timeline(replay: &mut Replay) {
    replay.step(
        command(
            "im_query_messages_by_channel",
            json!({
                "channel_id": CHANNEL_ID,
                "limit": 20,
                "windowToken": "latest",
            }),
        ),
        1_700_000_000_000,
    );
    let local_corr = replay.only_corr(CorrKind::Persist);
    replay.step(ok(local_corr, Bytes::from_static(b"[]")), 1_700_000_000_001);
    let remote_corr = replay.only_corr(CorrKind::Http);
    replay.step(
        ok(
            remote_corr,
            http_envelope(json!({"status": "SUCCESS", "data": []})),
        ),
        1_700_000_000_002,
    );
    assert_eq!(
        surface_names(&replay.emitted_frame()),
        BTreeSet::from(["message.timeline"])
    );

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
                "causationId": "g01-wdio-attach-1"
            }),
        ),
        1_700_000_000_003,
    );
    let _pending_initial_snapshot = replay.only_corr(CorrKind::Persist);
    // Attachment is registered before this readback completes. The already rendered snapshot
    // above makes the following P1 fact a causally linked patch rather than a synthetic snapshot.
}

fn message_row(id: &str, temporary_id: &str, status: &str) -> Value {
    json!({
        "id": id,
        "temporaryId": temporary_id,
        "channelId": CHANNEL_ID,
        "userId": "g01-wdio-actor",
        "createAt": CREATED_AT,
        "message": MARKER,
        "type": "TEXT",
        "sendStatus": status,
        "props": {},
    })
}

fn temporary_id(replay: &Replay) -> String {
    replay
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| {
            let Effect::Emit { event } = effect else {
                return None;
            };
            let envelope: Value = serde_json::from_slice(event.0.as_ref()).ok()?;
            (envelope["event"] == "im:post:sending")
                .then(|| envelope["data"]["temporaryId"].as_str().map(str::to_string))
                .flatten()
        })
        .expect("send command must expose the Helix-minted temporary identity")
}

fn produce(terminal: Terminal) -> (InteractionFrameV2, InteractionFrameV2) {
    let mut replay = Replay::new();
    attach_timeline(&mut replay);
    replay.step(
        command(
            "im_send_message",
            json!({
                "channel_id": CHANNEL_ID,
                "text": MARKER,
                "type": "TEXT",
                "req_id": REQUEST_ID,
            }),
        ),
        1_700_000_000_004,
    );
    let temporary_id = temporary_id(&replay);
    let p1_corr = replay.only_corr(CorrKind::Persist);

    replay.step(ok(p1_corr, Bytes::new()), 1_700_000_000_005);
    let sending_read_corr = replay.only_corr(CorrKind::Persist);
    replay.step(
        ok(
            sending_read_corr,
            Bytes::from(
                serde_json::to_vec(&json!([message_row(
                    &temporary_id,
                    &temporary_id,
                    "sending"
                )]))
                .expect("sending local readback serializes"),
            ),
        ),
        1_700_000_000_006,
    );
    let sending = replay.emitted_frame();
    let posts_create_corr = replay.only_corr(CorrKind::Http);

    match terminal {
        Terminal::Sent => replay.step(
            ok(
                posts_create_corr,
                http_envelope(json!({"status": "SUCCESS", "data": {"id": SERVER_ID}})),
            ),
            1_700_000_000_007,
        ),
        Terminal::Failed => replay.step(http_error(posts_create_corr), 1_700_000_000_007),
    }
    let terminal_persist_corr = replay.only_corr(CorrKind::Persist);
    replay.step(ok(terminal_persist_corr, Bytes::new()), 1_700_000_000_008);
    let terminal_read_corr = replay.only_corr(CorrKind::Persist);
    let status = match terminal {
        Terminal::Sent => "sent",
        Terminal::Failed => "unsend",
    };
    let id = match terminal {
        Terminal::Sent => SERVER_ID,
        Terminal::Failed => temporary_id.as_str(),
    };
    replay.step(
        ok(
            terminal_read_corr,
            Bytes::from(
                serde_json::to_vec(&json!([message_row(id, &temporary_id, status)]))
                    .expect("terminal local readback serializes"),
            ),
        ),
        1_700_000_000_009,
    );
    (sending, replay.emitted_frame())
}

fn surface_names(frame: &InteractionFrameV2) -> BTreeSet<&'static str> {
    frame
        .mutations
        .iter()
        .map(|mutation| mutation.mutation.surface.as_str())
        .collect()
}

fn delivery_kinds(frame: &InteractionFrameV2) -> BTreeSet<&str> {
    frame
        .mutations
        .iter()
        .filter_map(|mutation| match &mutation.mutation.mutation {
            TypedSurfaceMutation::TimelineSnapshot { view } => Some(&view.items),
            TypedSurfaceMutation::TimelinePatch { patch } => Some(&patch.view.items),
            _ => None,
        })
        .flatten()
        .map(|item| match item.delivery {
            helix_im::interaction_frame::DeliveryVm::Sending => "sending",
            helix_im::interaction_frame::DeliveryVm::Sent => "sent",
            helix_im::interaction_frame::DeliveryVm::Failed { .. } => "failed",
            helix_im::interaction_frame::DeliveryVm::Uploading { .. } => "uploading",
            helix_im::interaction_frame::DeliveryVm::Recalled => "recalled",
        })
        .collect()
}

fn canonical_bytes(frame: &InteractionFrameV2) -> Vec<u8> {
    let mut bytes = serde_json::to_vec_pretty(frame).expect("Frame serializes");
    bytes.push(b'\n');
    bytes
}

fn semantic_bytes(frame: &InteractionFrameV2) -> Vec<u8> {
    let mutation_kinds: Vec<&str> = frame
        .mutations
        .iter()
        .map(|mutation| match mutation.mutation.mutation {
            TypedSurfaceMutation::TimelineSnapshot { .. } => "timelineSnapshot",
            TypedSurfaceMutation::TimelinePatch { .. } => "timelinePatch",
            _ => "other",
        })
        .collect();
    serde_json::to_vec(&json!({
        "schemaVersion": frame.schema_version,
        "runtime": frame.runtime,
        "surfaces": surface_names(frame),
        "mutationKinds": mutation_kinds,
        "deliveryKinds": delivery_kinds(frame),
    }))
    .expect("semantic projection serializes")
}

fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("g01-wdio")
}

fn fixture_name(fixture_id: &str) -> &str {
    match fixture_id {
        "g01-sending-v2" => "sending.frame-v2.json",
        "g01-sent-v2" => "sent.frame-v2.json",
        "g01-failed-v2" => "failed.frame-v2.json",
        _ => panic!("unknown fixture id"),
    }
}

fn first_message(frame: &InteractionFrameV2) -> &helix_im::interaction_frame::MessageVm {
    frame
        .mutations
        .iter()
        .find_map(|mutation| match &mutation.mutation.mutation {
            TypedSurfaceMutation::TimelineSnapshot { view } => view.items.first(),
            TypedSurfaceMutation::TimelinePatch { patch } => patch.view.items.first(),
            _ => None,
        })
        .expect("G01 fixture contains its message identity")
}

fn first_message_id(frame: &InteractionFrameV2) -> &str {
    first_message(frame).id.as_str()
}

fn provenance_entry(
    fixture_id: &str,
    frame: &InteractionFrameV2,
    bytes: &[u8],
    temporary_id: &str,
) -> Value {
    json!({
        "fixtureId": fixture_id,
        "schemaVersion": frame.schema_version,
        "helixCommit": HELIX_COMMIT,
        "producer": PRODUCER,
        "sourceKind": "rust-export",
        "semanticSha256": sha256_hex(&semantic_bytes(frame)),
        "fixtureSha256": sha256_hex(bytes),
        "surfaces": surface_names(frame),
        "generatedAt": GENERATED_AT,
        "parameterizableFields": [
            "/transactionId",
            "/causationId",
            "/mutations/*/causationId",
            "/mutations/*/scope/channelId",
            "/mutations/*/mutation/view/channelId",
            "/mutations/*/mutation/view/anchor/messageId",
            "/mutations/*/mutation/view/items/*/id",
            "/mutations/*/mutation/view/items/*/channelId",
            "/mutations/*/mutation/view/items/*/author/userId",
            "/mutations/*/mutation/view/items/*/createdAt",
            "/mutations/*/mutation/view/items/*/content/text",
            "/mutations/*/mutation/patch/view/channelId",
            "/mutations/*/mutation/patch/view/anchor/messageId",
            "/mutations/*/mutation/patch/view/items/*/id",
            "/mutations/*/mutation/patch/view/items/*/channelId",
            "/mutations/*/mutation/patch/view/items/*/author/userId",
            "/mutations/*/mutation/patch/view/items/*/createdAt",
            "/mutations/*/mutation/patch/view/items/*/content/text"
        ],
        "placeholderValues": {
            "actorId": "g01-wdio-actor",
            "channelId": CHANNEL_ID,
            "createdAt": CREATED_AT,
            "marker": MARKER,
            "temporaryId": temporary_id,
            "postId": SERVER_ID,
            "reqId": REQUEST_ID,
            "transactionId": frame.transaction_id,
            "messageId": first_message_id(frame)
        }
    })
}

#[test]
fn current_g01_producer_matches_canonical_v2_fixtures_and_provenance() {
    let (sending_from_sent, sent) = produce(Terminal::Sent);
    let (sending_from_failed, failed) = produce(Terminal::Failed);
    assert_eq!(
        sending_from_sent, sending_from_failed,
        "sending fixture must not depend on the future terminal outcome"
    );

    let cases = [
        ("g01-sending-v2", sending_from_sent, "sending"),
        ("g01-sent-v2", sent, "sent"),
        ("g01-failed-v2", failed, "failed"),
    ];
    let temporary_id = first_message_id(&cases[0].1).to_string();
    assert_eq!(first_message_id(&cases[1].1), SERVER_ID);
    assert_eq!(first_message_id(&cases[2].1), temporary_id.as_str());
    for (_, frame, expected_delivery) in &cases {
        frame.validate().expect("producer Frame remains valid");
        assert_eq!(frame.schema_version, 2);
        assert_eq!(
            surface_names(frame),
            BTreeSet::from(["message.timeline"]),
            "G01 current producer must not gain fixture-only surfaces"
        );
        assert!(
            delivery_kinds(frame).contains(expected_delivery),
            "fixture must contain its expected delivery state"
        );
        assert!(
            frame
                .mutations
                .iter()
                .all(|mutation| mutation.mutation.surface == SurfaceId::MessageTimeline),
            "current ordinary G01 producer only emits message.timeline"
        );
        let message = first_message(frame);
        assert_eq!(message.channel_id, CHANNEL_ID);
        let helix_im::interaction_frame::MessageContentVm::Text { text } = &message.content else {
            panic!("G01 fixture must preserve the current text content contract")
        };
        assert_eq!(text, MARKER);
    }

    let directory = fixture_dir();
    let mut provenance = Vec::new();
    for (fixture_id, frame, _) in &cases {
        let bytes = canonical_bytes(frame);
        provenance.push(provenance_entry(
            fixture_id,
            frame,
            &bytes,
            temporary_id.as_str(),
        ));
        if std::env::var_os("UPDATE_G01_WDIO_FIXTURES").is_some() {
            fs::create_dir_all(&directory).expect("fixture directory is writable");
            fs::write(directory.join(fixture_name(fixture_id)), &bytes)
                .expect("fixture export succeeds");
        }
    }
    let mut provenance_bytes =
        serde_json::to_vec_pretty(&provenance).expect("provenance serializes");
    provenance_bytes.push(b'\n');
    if std::env::var_os("UPDATE_G01_WDIO_FIXTURES").is_some() {
        fs::write(directory.join("provenance.json"), &provenance_bytes)
            .expect("provenance export succeeds");
    }

    for (fixture_id, frame, _) in &cases {
        let path = directory.join(fixture_name(fixture_id));
        let checked_in = fs::read(&path).unwrap_or_else(|error| {
            panic!(
                "{} missing ({error}); run UPDATE_G01_WDIO_FIXTURES=1 cargo test -p helix-im --test g01_wdio_fixture_contract_test",
                path.display()
            )
        });
        assert_eq!(
            checked_in,
            canonical_bytes(frame),
            "{} drifted from current Helix producer",
            path.display()
        );
        InteractionFrameV2::decode(&checked_in)
            .expect("checked-in fixture remains decodable by current serializer");
    }
    assert_eq!(
        fs::read(directory.join("provenance.json")).expect("provenance exists"),
        provenance_bytes,
        "provenance hashes or producer identity drifted"
    );
}

// Test-only SHA-256 avoids adding a hash dependency to the runtime-agnostic helix-im crate.
fn sha256_hex(input: &[u8]) -> String {
    const INITIAL: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];

    let bit_len = (input.len() as u64) * 8;
    let mut padded = input.to_vec();
    padded.push(0x80);
    while padded.len() % 64 != 56 {
        padded.push(0);
    }
    padded.extend_from_slice(&bit_len.to_be_bytes());

    let mut state = INITIAL;
    for chunk in padded.chunks_exact(64) {
        let mut words = [0_u32; 64];
        for (index, word) in words.iter_mut().take(16).enumerate() {
            let offset = index * 4;
            *word = u32::from_be_bytes([
                chunk[offset],
                chunk[offset + 1],
                chunk[offset + 2],
                chunk[offset + 3],
            ]);
        }
        for index in 16..64 {
            let s0 = words[index - 15].rotate_right(7)
                ^ words[index - 15].rotate_right(18)
                ^ (words[index - 15] >> 3);
            let s1 = words[index - 2].rotate_right(17)
                ^ words[index - 2].rotate_right(19)
                ^ (words[index - 2] >> 10);
            words[index] = words[index - 16]
                .wrapping_add(s0)
                .wrapping_add(words[index - 7])
                .wrapping_add(s1);
        }

        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = state;
        for index in 0..64 {
            let sum1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let choose = (e & f) ^ ((!e) & g);
            let temp1 = h
                .wrapping_add(sum1)
                .wrapping_add(choose)
                .wrapping_add(K[index])
                .wrapping_add(words[index]);
            let sum0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let majority = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = sum0.wrapping_add(majority);
            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }
        for (slot, value) in state.iter_mut().zip([a, b, c, d, e, f, g, h].into_iter()) {
            *slot = slot.wrapping_add(value);
        }
    }
    state.iter().map(|word| format!("{word:08x}")).collect()
}
