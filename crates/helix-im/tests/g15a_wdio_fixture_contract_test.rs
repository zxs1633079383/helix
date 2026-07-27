//! G-15a WDIO/Mobile Frame V2 canonical Fixture 真源。
//!
//! 所有 JSON 来自 `G15aScenarioRunner -> ImModule::handle -> Effect::Emit`；普通测试只比较
//! actual 与 Golden，只有 `UPDATE_G15A_WDIO_FIXTURES=1` 才允许覆盖。

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use helix_im::interaction_frame::{InteractionFrameV2, TypedSurfaceMutation};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

#[path = "support/g15a_scenario.rs"]
mod scenario;

use scenario::{
    assert_created_contract, G15aScenarioRunner, ACTOR_ID, CHANNEL_ID, GROUP_NAME, OBSERVER_ID,
    REQUEST_ID, TEAM_ID,
};

const HELIX_COMMIT: &str = "157daca3760e9c94d7b6bc9f38798e199628b1aa";
const GENERATED_AT: &str = "2026-07-26T08:20:05-07:00";
const PRODUCER: &str =
    "G15aScenarioRunner -> ImModule::handle -> Effect::Emit -> InteractionFrameV2::decode";

/// 描述一份 G-15a canonical Fixture 的稳定 identity 和 producer 输出。
struct FixtureCase {
    fixture_id: &'static str,
    file_name: &'static str,
    frame: InteractionFrameV2,
}

/// 从唯一生产 runner 生成 terminal 与两个 durable recovery Frame。
fn produce_cases() -> Vec<FixtureCase> {
    let created = G15aScenarioRunner::run_http_created();
    let replayed = G15aScenarioRunner::run_persist_failure_replay();
    let recovered = G15aScenarioRunner::run_recovery();
    assert_created_contract(&created.created, Some(REQUEST_ID));
    assert_created_contract(&replayed, Some(REQUEST_ID));
    assert_eq!(
        created.created, replayed,
        "persist replay must converge on byte-equivalent terminal semantics"
    );
    vec![
        FixtureCase {
            fixture_id: "g15a-channel-created-v2",
            file_name: "channel-created.frame-v2.json",
            frame: created.created,
        },
        FixtureCase {
            fixture_id: "g15a-channel-summary-recovery-v2",
            file_name: "channel-summary-recovery.frame-v2.json",
            frame: recovered.summary,
        },
        FixtureCase {
            fixture_id: "g15a-member-roster-recovery-v2",
            file_name: "member-roster-recovery.frame-v2.json",
            frame: recovered.roster,
        },
    ]
}

/// 生成稳定 pretty JSON bytes 并固定末尾换行。
fn canonical_bytes(frame: &InteractionFrameV2) -> Vec<u8> {
    let mut bytes = serde_json::to_vec_pretty(frame).expect("G15a Frame serializes");
    bytes.push(b'\n');
    bytes
}

/// 提取排除 transaction/revision 后仍必须跨端一致的业务语义。
fn semantic_bytes(frame: &InteractionFrameV2) -> Vec<u8> {
    let views = frame
        .mutations
        .iter()
        .map(|mutation| match &mutation.mutation.mutation {
            TypedSurfaceMutation::ChannelSummarySnapshot { view } => json!({
                "kind": "channelSummarySnapshot",
                "channelId": view.channel_id,
                "title": view.title,
                "memberCount": view.member_count,
                "viewerRole": view.viewer_role,
                "lifecycle": view.lifecycle,
                "actions": view.actions.iter().map(|action| json!({
                    "kind": action.kind,
                    "inputShape": action.input_shape,
                    "enabled": action.enabled,
                    "disabledReason": action.disabled_reason,
                })).collect::<Vec<_>>(),
            }),
            TypedSurfaceMutation::MemberRosterSnapshot { view } => json!({
                "kind": "memberRosterSnapshot",
                "channelId": view.channel_id,
                "memberCount": view.member_count,
                "items": view.items.iter().map(|item| json!({
                    "user": item.user,
                    "nickname": item.nickname,
                    "role": item.role,
                    "presence": item.presence,
                    "membershipState": item.membership_state,
                    "actions": item.actions.iter().map(|action| json!({
                        "kind": action.kind,
                        "inputShape": action.input_shape,
                        "enabled": action.enabled,
                        "disabledReason": action.disabled_reason,
                    })).collect::<Vec<_>>(),
                })).collect::<Vec<_>>(),
                "page": view.page,
                "state": view.state,
            }),
            other => json!({"kind": format!("{other:?}")}),
        })
        .collect::<Vec<_>>();
    serde_json::to_vec(&json!({
        "schemaVersion": frame.schema_version,
        "runtime": frame.runtime,
        "surfaces": surface_names(frame),
        "views": views,
    }))
    .expect("G15a semantic projection serializes")
}

/// 返回 Frame 的稳定 surface inventory。
fn surface_names(frame: &InteractionFrameV2) -> BTreeSet<&'static str> {
    frame
        .mutations
        .iter()
        .map(|mutation| mutation.mutation.surface.as_str())
        .collect()
}

/// 计算 provenance 使用的小写 SHA-256。
fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

/// 返回 Helix canonical G-15a Fixture 目录。
fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("g15a-wdio")
}

/// 为一份 Fixture 生成真源、hash、surface 与唯一允许的动态字段白名单。
fn provenance_entry(case: &FixtureCase, bytes: &[u8]) -> Value {
    json!({
        "fixtureId": case.fixture_id,
        "fileName": case.file_name,
        "gateId": "G-15a",
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
            "/mutations/*/scope/cursor",
            "/mutations/*/mutation/view/channelId",
            "/mutations/*/mutation/view/title/text",
            "/mutations/*/mutation/view/items/*/user/userId",
            "/mutations/*/mutation/view/actions/*/surfaceEpoch",
            "/mutations/*/mutation/view/items/*/actions/*/surfaceEpoch"
        ],
        "rebaseFields": [
            "/transactionId",
            "/mutations/*/surfaceEpoch",
            "/mutations/*/revision",
            "/mutations/*/baseRevision",
            "/mutations/*/mutation/view/actions/*/surfaceEpoch",
            "/mutations/*/mutation/view/items/*/actions/*/surfaceEpoch"
        ],
        "placeholderValues": {
            "actorId": ACTOR_ID,
            "observerId": OBSERVER_ID,
            "teamId": TEAM_ID,
            "channelId": CHANNEL_ID,
            "groupName": GROUP_NAME,
            "reqId": REQUEST_ID,
            "transactionId": case.frame.transaction_id,
            "surfaceEpoch": case.frame.mutations[0].surface_epoch,
            "revision": case.frame.mutations[0].mutation.revision,
            "baseRevision": case.frame.mutations[0].mutation.base_revision
        }
    })
}

#[test]
/// 证明 actual producer、完整字段、Golden bytes 与 provenance 始终同源。
fn current_g15a_producer_matches_canonical_v2_fixtures_and_provenance() {
    let cases = produce_cases();
    assert_eq!(cases.len(), 3, "G15a canonical inventory is explicit");
    assert_eq!(
        surface_names(&cases[0].frame),
        BTreeSet::from(["message.channel-summary", "message.member-roster",])
    );
    let directory = fixture_dir();
    let update = std::env::var_os("UPDATE_G15A_WDIO_FIXTURES").is_some();
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
        serde_json::to_vec_pretty(&provenance).expect("G15a provenance serializes");
    provenance_bytes.push(b'\n');
    if update {
        fs::write(directory.join("provenance.json"), &provenance_bytes)
            .expect("provenance export succeeds");
    }
    for case in &cases {
        let path = directory.join(case.file_name);
        let checked_in = fs::read(&path).unwrap_or_else(|error| {
            panic!(
                "{} missing ({error}); run UPDATE_G15A_WDIO_FIXTURES=1 cargo test -p helix-im --test g15a_wdio_fixture_contract_test",
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
        "G15a provenance hashes or producer identity drifted"
    );
}
