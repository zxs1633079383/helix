//! G05 WDIO/Mobile Frame V2 canonical Fixture 真源与字段级合同。
//!
//! 普通测试只比较 production actual 与 checked-in Golden；只有显式
//! `UPDATE_G05_WDIO_FIXTURES=1` 才允许覆盖跨仓 canonical JSON。

use std::fs;
use std::path::{Path, PathBuf};

use helix_im::interaction_frame::{InteractionFrameV2, TypedSurfaceMutation};
use serde_json::{json, Value};

#[path = "support/g05_scenario.rs"]
#[allow(dead_code)]
mod scenario;

use scenario::{
    assert_source_forwardable_contract, assert_target_dialog_contract,
    assert_target_forwarded_contract, first_dialog_row, first_message, surface_names,
    G05ScenarioRunner, ACTOR_ID, CREATED_AT, MARKER, OBSERVER_ID, REQUEST_ID, SOURCE_CHANNEL_ID,
    SOURCE_GROUP_NAME, SOURCE_POST_ID, TARGET_CHANNEL_ID, TARGET_GROUP_NAME, TARGET_POST_ID,
    TEAM_ID,
};

const HELIX_COMMIT: &str = "3798ffdd0bfd89b8922829dd252327bb0a2075ec";
const GENERATED_AT: &str = "2026-07-25T13:30:00-07:00";
const PRODUCER: &str =
    "G05ScenarioRunner -> ImModule::handle -> Effect::Emit -> InteractionFrameV2::decode";

/// 描述一份稳定 Fixture 的 ID、文件名和实际 producer Frame。
struct FixtureCase {
    fixture_id: &'static str,
    file_name: &'static str,
    frame: InteractionFrameV2,
}

/// 从唯一 ScenarioRunner 生成 fresh group、source、target timeline 与 dialog 四份 Frame。
fn produce_cases() -> Vec<FixtureCase> {
    let trace = G05ScenarioRunner::run_happy_ws_first();
    assert_eq!(trace.http_accepted_frame_count, 0);
    assert_eq!(trace.late_http_frame_count, 0);
    assert_eq!(
        first_dialog_row(&trace.group_created)
            .expect("fresh target group exists")
            .channel_id,
        TARGET_CHANNEL_ID
    );
    assert_source_forwardable_contract(&trace.source_forwardable);
    assert_target_forwarded_contract(&trace.target_forwarded);
    assert_target_dialog_contract(&trace.target_dialog);
    vec![
        FixtureCase {
            fixture_id: "g05-group-created-v2",
            file_name: "group-created.frame-v2.json",
            frame: trace.group_created,
        },
        FixtureCase {
            fixture_id: "g05-source-forwardable-v2",
            file_name: "source-forwardable.frame-v2.json",
            frame: trace.source_forwardable,
        },
        FixtureCase {
            fixture_id: "g05-target-forwarded-v2",
            file_name: "target-forwarded.frame-v2.json",
            frame: trace.target_forwarded,
        },
        FixtureCase {
            fixture_id: "g05-target-dialog-v2",
            file_name: "target-dialog.frame-v2.json",
            frame: trace.target_dialog,
        },
    ]
}

/// 生成稳定 pretty JSON bytes 并固定末尾换行。
fn canonical_bytes(frame: &InteractionFrameV2) -> Vec<u8> {
    let mut bytes = serde_json::to_vec_pretty(frame).expect("Frame serializes");
    bytes.push(b'\n');
    bytes
}

/// 生成排除 transaction 身份但保留 G05 业务语义的摘要。
fn semantic_bytes(frame: &InteractionFrameV2) -> Vec<u8> {
    let mutation_kinds = frame
        .mutations
        .iter()
        .map(|mutation| match mutation.mutation.mutation {
            TypedSurfaceMutation::TimelineSnapshot { .. } => "timelineSnapshot",
            TypedSurfaceMutation::TimelineDelta { .. } => "timelineDelta",
            TypedSurfaceMutation::DialogListSnapshot { .. } => "dialogListSnapshot",
            TypedSurfaceMutation::DialogListPatch { .. } => "dialogListPatch",
            _ => "other",
        })
        .collect::<Vec<_>>();
    serde_json::to_vec(&json!({
        "schemaVersion": frame.schema_version,
        "runtime": frame.runtime,
        "causationId": frame.causation_id,
        "surfaces": surface_names(frame),
        "mutationKinds": mutation_kinds,
        "message": message_contract(frame),
        "dialog": dialog_contract(frame),
    }))
    .expect("semantic projection serializes")
}

/// 提取 timeline content、event sequence、delivery 与 action kinds。
fn message_contract(frame: &InteractionFrameV2) -> Option<Value> {
    let message = first_message(frame)?;
    Some(json!({
        "id": message.id,
        "channelId": message.channel_id,
        "eventSeq": message.event_seq,
        "content": message.content,
        "delivery": message.delivery,
        "actions": message.actions.iter().map(|action| json!({
            "kind": action.kind,
            "enabled": action.enabled,
            "disabledReason": action.disabled_reason,
        })).collect::<Vec<_>>(),
        "quickActions": message.quick_actions,
    }))
}

/// 提取 fresh/terminal dialog 的 target channel、preview 与 activity。
fn dialog_contract(frame: &InteractionFrameV2) -> Option<Value> {
    let row = first_dialog_row(frame)?;
    Some(json!({
        "channelId": row.channel_id,
        "title": row.title,
        "preview": row.preview,
        "lastActivityAt": row.last_activity_at,
        "unreadCount": row.unread_count,
        "lifecycle": row.lifecycle,
    }))
}

/// 返回 Helix canonical G05 Fixture 目录。
fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("g05-wdio")
}

/// 为单份 Fixture 生成 producer、hash、surface 与动态字段白名单。
fn provenance_entry(case: &FixtureCase, bytes: &[u8]) -> Value {
    json!({
        "fixtureId": case.fixture_id,
        "fileName": case.file_name,
        "schemaVersion": case.frame.schema_version,
        "helixCommit": HELIX_COMMIT,
        "producer": PRODUCER,
        "sourceKind": "rust-export",
        "semanticSha256": sha256_hex(&semantic_bytes(&case.frame)),
        "fixtureSha256": sha256_hex(bytes),
        "surfaces": surface_names(&case.frame),
        "generatedAt": GENERATED_AT,
        "parameterizableFields": [
            "/transactionId",
            "/causationId",
            "/mutations/*/causationId",
            "/mutations/*/surfaceEpoch",
            "/mutations/*/revision",
            "/mutations/*/baseRevision",
            "/mutations/*/scope/channelId",
            "/mutations/*/scope/userId",
            "/mutations/*/scope/teamId",
            "/mutations/*/mutation/view/channelId",
            "/mutations/*/mutation/view/items/*/id",
            "/mutations/*/mutation/view/items/*/channelId",
            "/mutations/*/mutation/view/items/*/createdAt",
            "/mutations/*/mutation/view/items/*/content/text",
            "/mutations/*/mutation/view/items/*/actions/*/surfaceEpoch",
            "/mutations/*/mutation/patch/view/channelId",
            "/mutations/*/mutation/patch/view/items/*/id",
            "/mutations/*/mutation/patch/view/items/*/channelId",
            "/mutations/*/mutation/patch/view/items/*/createdAt",
            "/mutations/*/mutation/patch/view/items/*/content/text",
            "/mutations/*/mutation/patch/view/items/*/actions/*/surfaceEpoch",
            "/mutations/*/mutation/patch/view/sections/*/items/*/channelId",
            "/mutations/*/mutation/patch/view/sections/*/items/*/lastActivityAt",
            "/mutations/*/mutation/patch/view/sections/*/items/*/preview/text",
            "/mutations/*/mutation/patch/view/sections/*/items/*/title/text",
            "/mutations/*/mutation/view/sections/*/items/*/channelId",
            "/mutations/*/mutation/view/sections/*/items/*/lastActivityAt",
            "/mutations/*/mutation/view/sections/*/items/*/preview/text",
            "/mutations/*/mutation/view/sections/*/items/*/title/text"
        ],
        "rebaseFields": [
            "/transactionId",
            "/mutations/*/surfaceEpoch",
            "/mutations/*/revision",
            "/mutations/*/baseRevision",
            "/mutations/*/mutation/view/items/*/actions/*/surfaceEpoch",
            "/mutations/*/mutation/patch/view/items/*/actions/*/surfaceEpoch"
        ],
        "placeholderValues": {
            "actorId": ACTOR_ID,
            "observerId": OBSERVER_ID,
            "teamId": TEAM_ID,
            "sourceChannelId": SOURCE_CHANNEL_ID,
            "targetChannelId": TARGET_CHANNEL_ID,
            "sourceGroupName": SOURCE_GROUP_NAME,
            "targetGroupName": TARGET_GROUP_NAME,
            "createdAt": CREATED_AT,
            "marker": MARKER,
            "sourcePostId": SOURCE_POST_ID,
            "targetPostId": TARGET_POST_ID,
            "reqId": REQUEST_ID,
            "transactionId": case.frame.transaction_id,
            "surfaceEpoch": case.frame.mutations[0].surface_epoch,
            "revision": case.frame.mutations[0].mutation.revision,
            "baseRevision": case.frame.mutations[0].mutation.base_revision
        }
    })
}

#[test]
/// 验证 actual Frame、字段语义、provenance 与 checked-in Golden 字节完全一致。
fn g05_canonical_frames_match_checked_in_contract() {
    let update = std::env::var("UPDATE_G05_WDIO_FIXTURES").as_deref() == Ok("1");
    let directory = fixture_dir();
    let cases = produce_cases();
    assert_eq!(cases.len(), 4, "G05 canonical inventory is fixed");
    let mut provenance = Vec::with_capacity(cases.len());
    for case in &cases {
        case.frame.validate().expect("producer Frame remains valid");
        let expected_schema_version = if case
            .frame
            .mutations
            .iter()
            .any(|mutation| mutation.mutation.surface.as_str() == "message.timeline")
        {
            3
        } else {
            2
        };
        assert_eq!(case.frame.schema_version, expected_schema_version);
        assert!(!surface_names(&case.frame).is_empty());
        let bytes = canonical_bytes(&case.frame);
        provenance.push(provenance_entry(case, &bytes));
        if update {
            fs::create_dir_all(&directory).expect("fixture directory is writable");
            fs::write(directory.join(case.file_name), &bytes).expect("fixture export succeeds");
        }
    }
    let mut provenance_bytes =
        serde_json::to_vec_pretty(&provenance).expect("provenance serializes");
    provenance_bytes.push(b'\n');
    if update {
        fs::write(directory.join("provenance.json"), &provenance_bytes)
            .expect("provenance export succeeds");
    }
    for case in &cases {
        let path = directory.join(case.file_name);
        let checked_in = fs::read(&path).unwrap_or_else(|error| {
            panic!(
                "{} missing ({error}); run UPDATE_G05_WDIO_FIXTURES=1 cargo test -p helix-im --test g05_wdio_fixture_contract_test",
                path.display()
            )
        });
        assert_eq!(
            checked_in,
            canonical_bytes(&case.frame),
            "{} drifted from current Helix producer",
            path.display()
        );
        InteractionFrameV2::decode(&checked_in).expect("checked-in fixture remains current V2");
    }
    assert_eq!(
        fs::read(directory.join("provenance.json")).expect("provenance exists"),
        provenance_bytes,
        "provenance hashes or producer identity drifted"
    );
}

/// 计算测试资产 SHA-256，避免给 runtime-agnostic crate 增加 hash 依赖。
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
