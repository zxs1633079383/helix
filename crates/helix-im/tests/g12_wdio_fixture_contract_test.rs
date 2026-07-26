//! G12 WDIO/Mobile Frame V2 canonical Fixture 真源。
//!
//! 普通测试只比较 production producer 与 checked-in Golden；仅
//! `UPDATE_G12_WDIO_FIXTURES=1` 允许覆盖 canonical JSON。

use std::fs;
use std::path::{Path, PathBuf};

use helix_im::interaction_frame::{InteractionFrameV2, TypedSurfaceMutation};
use serde_json::{json, Value};

#[path = "support/g12_scenario.rs"]
#[allow(dead_code)]
mod scenario;

use scenario::{
    assert_branch_contract, assert_group_created_contract_for, assert_snapshot_contract,
    g12_scenario_spec, sha256_hex, surface_names, G12ScenarioRunner, ACTOR_ID, BRANCH_REPLY_ID,
    CHANNEL_ID, CREATED_AT, FIRST_REPLY_ID, GROUP_NAME, OBSERVER_ID, ROOT_ID, SECOND_REPLY_ID,
    TEAM_ID,
};

const HELIX_COMMIT: &str = "a2207d73f8e72da7374b916b2167ee3a25b9f6c3";
const GENERATED_AT: &str = "2026-07-26T07:55:00-07:00";
const PRODUCER: &str =
    "G12ScenarioRunner -> ImModule::handle -> Effect::Emit -> InteractionFrameV2::decode";

/// 绑定稳定 fixture id、文件名与 production 回放得到的完整 Frame。
struct FixtureCase {
    fixture_id: &'static str,
    file_name: &'static str,
    frame: InteractionFrameV2,
}

/// 从唯一 G12 ScenarioRunner 生成建群、thread snapshot 与 branch patch。
fn produce_cases() -> Vec<FixtureCase> {
    let success = G12ScenarioRunner::run_success();
    assert_group_created_contract_for(&success.group_created, &g12_scenario_spec());
    assert_snapshot_contract(&success.snapshot);
    assert_branch_contract(&success.branch);
    vec![
        FixtureCase {
            fixture_id: "g12-group-created-v2",
            file_name: "group-created.frame-v2.json",
            frame: success.group_created,
        },
        FixtureCase {
            fixture_id: "g12-thread-snapshot-v2",
            file_name: "thread-snapshot.frame-v2.json",
            frame: success.snapshot,
        },
        FixtureCase {
            fixture_id: "g12-thread-branch-v2",
            file_name: "thread-branch.frame-v2.json",
            frame: success.branch,
        },
    ]
}

/// 生成稳定 pretty JSON bytes，并固定末尾换行。
fn canonical_bytes(frame: &InteractionFrameV2) -> Vec<u8> {
    let mut bytes = serde_json::to_vec_pretty(frame).expect("G12 Frame serializes");
    bytes.push(b'\n');
    bytes
}

/// 提取排除运行期 identity 后仍须跨端一致的 G12 业务字段。
fn semantic_bytes(frame: &InteractionFrameV2) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "schemaVersion": frame.schema_version,
        "runtime": frame.runtime,
        "surfaces": surface_names(frame),
        "thread": thread_contract(frame),
    }))
    .expect("G12 semantic projection serializes")
}

/// 提取 thread 的稳定 context、顺序、节点内容、分页与动作能力。
fn thread_contract(frame: &InteractionFrameV2) -> Option<Value> {
    let view = frame
        .mutations
        .iter()
        .find_map(|mutation| match &mutation.mutation.mutation {
            TypedSurfaceMutation::ThreadDetailSnapshot { view } => Some(view),
            TypedSurfaceMutation::ThreadDetailPatch { patch } => Some(&patch.view),
            _ => None,
        })?;
    Some(json!({
        "detailContext": view.detail_context,
        "root": message_contract(&view.root),
        "orderedReplyIds": view.ordered_reply_ids,
        "nodes": view.nodes.values().map(message_contract).collect::<Vec<_>>(),
        "replyCount": view.reply_count,
        "page": view.page,
        "state": view.state,
    }))
}

/// 提取消息节点的稳定作者、内容、交付态与动作合同。
fn message_contract(message: &helix_im::interaction_frame::MessageVm) -> Value {
    json!({
        "author": message.author,
        "content": message.content,
        "delivery": message.delivery,
        "replySummary": message.reply_summary,
        "actions": message.actions.iter().map(|action| json!({
            "kind": action.kind,
            "inputShape": action.input_shape,
            "enabled": action.enabled,
            "disabledReason": action.disabled_reason,
        })).collect::<Vec<_>>(),
    })
}

/// 返回 Helix canonical G12 Fixture 目录。
fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("g12-wdio")
}

/// 为一份 G12 Fixture 生成 producer、hash、surface 与动态字段白名单。
fn provenance_entry(case: &FixtureCase, bytes: &[u8]) -> Value {
    let mutation = &case.frame.mutations[0];
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
            "/mutations/*/scope/rootMessageId",
            "/mutations/*/scope/teamId",
            "/mutations/*/mutation/view/channelId",
            "/mutations/*/mutation/view/root/id",
            "/mutations/*/mutation/view/root/channelId",
            "/mutations/*/mutation/view/root/author/userId",
            "/mutations/*/mutation/view/root/createdAt",
            "/mutations/*/mutation/view/root/content/text",
            "/mutations/*/mutation/view/root/actions/*/actionRef",
            "/mutations/*/mutation/view/root/actions/*/surfaceEpoch",
            "/mutations/*/mutation/view/orderedReplyIds/*",
            "/mutations/*/mutation/view/nodes/*/id",
            "/mutations/*/mutation/view/nodes/*/channelId",
            "/mutations/*/mutation/view/nodes/*/author/userId",
            "/mutations/*/mutation/view/nodes/*/createdAt",
            "/mutations/*/mutation/view/nodes/*/content/text",
            "/mutations/*/mutation/view/nodes/*/actions/*/actionRef",
            "/mutations/*/mutation/view/nodes/*/actions/*/surfaceEpoch",
            "/mutations/*/mutation/patch/view/channelId",
            "/mutations/*/mutation/patch/view/root/id",
            "/mutations/*/mutation/patch/view/root/channelId",
            "/mutations/*/mutation/patch/view/root/author/userId",
            "/mutations/*/mutation/patch/view/root/createdAt",
            "/mutations/*/mutation/patch/view/root/content/text",
            "/mutations/*/mutation/patch/view/root/actions/*/actionRef",
            "/mutations/*/mutation/patch/view/root/actions/*/surfaceEpoch",
            "/mutations/*/mutation/patch/view/orderedReplyIds/*",
            "/mutations/*/mutation/patch/view/nodes/*/id",
            "/mutations/*/mutation/patch/view/nodes/*/channelId",
            "/mutations/*/mutation/patch/view/nodes/*/author/userId",
            "/mutations/*/mutation/patch/view/nodes/*/createdAt",
            "/mutations/*/mutation/patch/view/nodes/*/content/text",
            "/mutations/*/mutation/patch/view/nodes/*/actions/*/actionRef",
            "/mutations/*/mutation/patch/view/nodes/*/actions/*/surfaceEpoch",
            "/mutations/*/mutation/view/sections/*/items/*/channelId",
            "/mutations/*/mutation/view/sections/*/items/*/lastActivityAt",
            "/mutations/*/mutation/view/sections/*/items/*/title/text",
            "/mutations/*/mutation/view/sections/*/items/*/avatar/userIds/*",
            "/mutations/*/mutation/view/sections/*/items/*/avatar/fallbackText"
        ],
        "rebaseFields": [
            "/transactionId",
            "/mutations/*/surfaceEpoch",
            "/mutations/*/revision",
            "/mutations/*/baseRevision",
            "/mutations/*/mutation/view/root/actions/*/surfaceEpoch",
            "/mutations/*/mutation/view/nodes/*/actions/*/surfaceEpoch",
            "/mutations/*/mutation/patch/view/root/actions/*/surfaceEpoch",
            "/mutations/*/mutation/patch/view/nodes/*/actions/*/surfaceEpoch"
        ],
        "placeholderValues": {
            "actorId": ACTOR_ID,
            "observerId": OBSERVER_ID,
            "teamId": TEAM_ID,
            "channelId": CHANNEL_ID,
            "rootMessageId": ROOT_ID,
            "firstReplyId": FIRST_REPLY_ID,
            "secondReplyId": SECOND_REPLY_ID,
            "branchReplyId": BRANCH_REPLY_ID,
            "groupName": GROUP_NAME,
            "createdAt": CREATED_AT,
            "transactionId": case.frame.transaction_id,
            "surfaceEpoch": mutation.surface_epoch,
            "revision": mutation.mutation.revision,
            "baseRevision": mutation.mutation.base_revision
        }
    })
}

/// 证明 actual producer、字段语义、Golden bytes 与 provenance 始终同源。
#[test]
fn current_g12_producer_matches_canonical_v2_fixtures_and_provenance() {
    let cases = produce_cases();
    assert_eq!(cases.len(), 3, "G12 canonical inventory is explicit");
    let directory = fixture_dir();
    let update = std::env::var_os("UPDATE_G12_WDIO_FIXTURES").is_some();
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
        serde_json::to_vec_pretty(&provenance).expect("G12 provenance serializes");
    provenance_bytes.push(b'\n');
    if update {
        fs::write(directory.join("provenance.json"), &provenance_bytes)
            .expect("provenance export succeeds");
    }
    for case in &cases {
        let path = directory.join(case.file_name);
        let checked_in = fs::read(&path).unwrap_or_else(|error| {
            panic!(
                "{} missing ({error}); run UPDATE_G12_WDIO_FIXTURES=1 cargo test -p helix-im --test g12_wdio_fixture_contract_test",
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
        "G12 provenance hashes or producer identity drifted"
    );
}
