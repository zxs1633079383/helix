//! G14 WDIO/Mobile Frame V2 canonical Fixture 真源。
//!
//! 普通测试只比较 production producer 与 checked-in Golden；仅
//! `UPDATE_G14_WDIO_FIXTURES=1` 允许覆盖 canonical JSON。

use std::fs;
use std::path::{Path, PathBuf};

use helix_im::interaction_frame::{InteractionFrameV2, TypedSurfaceMutation};
use serde_json::{json, Value};

#[path = "support/g14_scenario.rs"]
#[allow(dead_code)]
mod scenario;

use scenario::{
    assert_group_created_contract_for, assert_readable_message_contract,
    assert_receipt_contract, assert_receipt_viewable_message_contract, g14_scenario_spec,
    receipt_view, sha256_hex, surface_names, G14ScenarioRunner, ACTOR_ID, AUTHOR_ID, CHANNEL_ID,
    CREATED_AT, GROUP_NAME, OBSERVER_ID, POST_ID, TEAM_ID,
};

const HELIX_COMMIT: &str = "5b8b7db031faf55f1fa52de15bac746a20f82b13";
const GENERATED_AT: &str = "2026-07-26T04:35:00-07:00";
const PRODUCER: &str =
    "G14ScenarioRunner -> ImModule::handle -> Effect::Emit -> InteractionFrameV2::decode";

/// 绑定稳定 fixture id、文件名与 production 回放得到的完整 Frame。
struct FixtureCase {
    fixture_id: &'static str,
    file_name: &'static str,
    frame: InteractionFrameV2,
}

/// 从唯一 G14 ScenarioRunner 生成群聊、可读消息、pending、terminal 与 recovery Frame。
fn produce_cases() -> Vec<FixtureCase> {
    let success = G14ScenarioRunner::run_success();
    let recovery = G14ScenarioRunner::run_recovery();
    assert_group_created_contract_for(&success.group_created, &g14_scenario_spec());
    assert_readable_message_contract(&success.readable_message);
    assert_receipt_viewable_message_contract(&success.receipt_viewable_message);
    assert_receipt_contract(&success.initial, 1, false);
    assert_receipt_contract(&success.terminal, 2, true);
    assert_receipt_contract(&recovery, 1, true);
    vec![
        FixtureCase {
            fixture_id: "g14-group-created-v2",
            file_name: "group-created.frame-v2.json",
            frame: success.group_created,
        },
        FixtureCase {
            fixture_id: "g14-readable-message-v2",
            file_name: "readable-message.frame-v2.json",
            frame: success.readable_message,
        },
        FixtureCase {
            fixture_id: "g14-receipt-viewable-message-v2",
            file_name: "receipt-viewable-message.frame-v2.json",
            frame: success.receipt_viewable_message,
        },
        FixtureCase {
            fixture_id: "g14-receipts-pending-v2",
            file_name: "receipts-pending.frame-v2.json",
            frame: success.initial,
        },
        FixtureCase {
            fixture_id: "g14-post-read-completed-v2",
            file_name: "post-read-completed.frame-v2.json",
            frame: success.terminal,
        },
        FixtureCase {
            fixture_id: "g14-post-read-recovery-v2",
            file_name: "post-read-recovery.frame-v2.json",
            frame: recovery,
        },
    ]
}

/// 生成稳定 pretty JSON bytes，并固定末尾换行。
fn canonical_bytes(frame: &InteractionFrameV2) -> Vec<u8> {
    let mut bytes = serde_json::to_vec_pretty(frame).expect("G14 Frame serializes");
    bytes.push(b'\n');
    bytes
}

/// 提取排除运行期 identity 后仍须跨端一致的 G14 业务字段。
fn semantic_bytes(frame: &InteractionFrameV2) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "schemaVersion": frame.schema_version,
        "runtime": frame.runtime,
        "surfaces": surface_names(frame),
        "readableMessage": readable_message_contract(frame),
        "receipts": receipt_contract(frame),
    }))
    .expect("G14 semantic projection serializes")
}

/// 提取 G14 可读消息的稳定内容与 markRead 能力，不纳入 opaque actionRef。
fn readable_message_contract(frame: &InteractionFrameV2) -> Value {
    let Some(view) = frame.mutations.iter().find_map(|mutation| match &mutation.mutation.mutation {
        TypedSurfaceMutation::TimelineSnapshot { view } => Some(view),
        TypedSurfaceMutation::TimelinePatch { patch } => Some(&patch.view),
        _ => None,
    }) else {
        return Value::Null;
    };
    json!({
        "channelId": view.channel_id,
        "items": view.items.iter().map(|item| json!({
            "author": item.author,
            "content": item.content,
            "delivery": item.delivery,
            "actions": (["markRead", "viewReceipt"].iter().map(|kind| {
                item.actions.iter().find(|action| action.kind == *kind).map(|action| json!({
                    "kind": action.kind,
                    "inputShape": action.input_shape,
                    "enabled": action.enabled,
                    "disabledReason": action.disabled_reason,
                }))
            }).collect::<Vec<_>>())
        })).collect::<Vec<_>>(),
        "state": view.state,
    })
}

/// 提取 participant receipt summary、completed 与 pending 的稳定字段。
fn receipt_contract(frame: &InteractionFrameV2) -> Value {
    let Some(view) =
        frame
            .mutations
            .iter()
            .find_map(|mutation| match &mutation.mutation.mutation {
                TypedSurfaceMutation::ParticipantReceiptsSnapshot { view } => Some(view),
                TypedSurfaceMutation::ParticipantReceiptsPatch { patch } => Some(&patch.view),
                _ => None,
            })
    else {
        return Value::Null;
    };
    json!({
        "channelId": view.channel_id,
        "messageId": view.message_id,
        "kind": view.kind,
        "summary": view.summary,
        "completed": view.completed,
        "pending": view.pending,
        "state": view.state,
    })
}

/// 返回 Helix canonical G14 Fixture 目录。
fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("g14-wdio")
}

/// 为一份 G14 Fixture 生成 producer、hash、surface 与动态字段白名单。
fn provenance_entry(case: &FixtureCase, bytes: &[u8]) -> Value {
    let first_mutation = case
        .frame
        .mutations
        .first()
        .expect("G14 fixture always contains at least one mutation");
    json!({
        "fixtureId": case.fixture_id,
        "fileName": case.file_name,
        "gateId": "G-14",
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
            "/mutations/*/scope/messageId",
            "/mutations/*/mutation/view/channelId",
            "/mutations/*/mutation/view/messageId",
            "/mutations/*/mutation/view/completed/*/user/userId",
            "/mutations/*/mutation/view/pending/*/user/userId",
            "/mutations/*/mutation/patch/view/channelId",
            "/mutations/*/mutation/patch/view/messageId",
            "/mutations/*/mutation/patch/view/completed/*/user/userId",
            "/mutations/*/mutation/patch/view/pending/*/user/userId",
            "/mutations/*/mutation/view/items/*/channelId",
            "/mutations/*/mutation/view/items/*/id",
            "/mutations/*/mutation/view/items/*/author/userId",
            "/mutations/*/mutation/patch/view/items/*/channelId",
            "/mutations/*/mutation/patch/view/items/*/id",
            "/mutations/*/mutation/patch/view/items/*/author/userId",
            "/mutations/*/mutation/view/sections/*/items/*/channelId",
            "/mutations/*/mutation/view/sections/*/items/*/title/text",
            "/mutations/*/mutation/view/sections/*/items/*/avatar/userIds/*",
            "/mutations/*/mutation/view/sections/*/items/*/avatar/fallbackText",
            "/mutations/*/mutation/patch/view/sections/*/items/*/channelId",
            "/mutations/*/mutation/patch/view/sections/*/items/*/title/text",
            "/mutations/*/mutation/patch/view/sections/*/items/*/avatar/userIds/*",
            "/mutations/*/mutation/patch/view/sections/*/items/*/avatar/fallbackText"
        ],
        "rebaseFields": [
            "/transactionId",
            "/mutations/*/surfaceEpoch",
            "/mutations/*/revision",
            "/mutations/*/baseRevision"
        ],
        "placeholderValues": {
            "actorId": ACTOR_ID,
            "observerId": OBSERVER_ID,
            "authorId": AUTHOR_ID,
            "teamId": TEAM_ID,
            "channelId": CHANNEL_ID,
            "postId": POST_ID,
            "groupName": GROUP_NAME,
            "createdAt": CREATED_AT,
            "transactionId": case.frame.transaction_id,
            "surfaceEpoch": first_mutation.surface_epoch,
            "revision": first_mutation.mutation.revision,
            "baseRevision": first_mutation.mutation.base_revision
        }
    })
}

/// 证明 actual producer、字段语义、Golden bytes 与 provenance 始终同源。
#[test]
fn current_g14_producer_matches_canonical_v2_fixtures_and_provenance() {
    let cases = produce_cases();
    assert_eq!(cases.len(), 6, "G14 canonical inventory is explicit");
    assert_eq!(receipt_view(&cases[3].frame).pending.len(), 2);
    assert_eq!(receipt_view(&cases[4].frame).completed.len(), 1);
    let directory = fixture_dir();
    let update = std::env::var_os("UPDATE_G14_WDIO_FIXTURES").is_some();
    let mut provenance = Vec::with_capacity(cases.len());
    for case in &cases {
        case.frame.validate().expect("producer Frame remains valid");
        assert_eq!(case.frame.schema_version, 2);
        let bytes = canonical_bytes(&case.frame);
        provenance.push(provenance_entry(case, &bytes));
        if update {
            fs::create_dir_all(&directory).expect("fixture directory is writable");
            fs::write(directory.join(case.file_name), &bytes).expect("fixture export succeeds");
        }
    }
    let mut provenance_bytes =
        serde_json::to_vec_pretty(&provenance).expect("G14 provenance serializes");
    provenance_bytes.push(b'\n');
    if update {
        fs::write(directory.join("provenance.json"), &provenance_bytes)
            .expect("provenance export succeeds");
    }
    for case in &cases {
        let path = directory.join(case.file_name);
        let checked_in = fs::read(&path).unwrap_or_else(|error| {
            panic!(
                "{} missing ({error}); run UPDATE_G14_WDIO_FIXTURES=1 cargo test -p helix-im --test g14_wdio_fixture_contract_test",
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
        "G14 provenance hashes or producer identity drifted"
    );
}
