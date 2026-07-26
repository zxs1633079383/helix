//! G08 WDIO/Mobile Frame V2 canonical Fixture 真源。
//!
//! 普通测试只比较真实 producer 与 checked-in Golden；仅
//! `UPDATE_G08_WDIO_FIXTURES=1` 允许覆盖 canonical JSON。

use std::fs;
use std::path::{Path, PathBuf};

use helix_im::interaction_frame::{InteractionFrameV2, TypedSurfaceMutation};
use serde_json::{json, Value};

#[path = "support/g08_scenario.rs"]
mod scenario;

use scenario::{
    assert_group_created_contract_for, assert_schedule_summary_contract, g08_scenario_spec,
    sha256_hex, surface_names, G08ScenarioRunner, ACTOR_ID, CHANNEL_ID, CREATED_AT, GROUP_NAME,
    MARKER, OBSERVER_ID, REQUEST_ID, TEAM_ID,
};

const HELIX_COMMIT: &str = "8358082b3fd01b96b332c79deca8d2017315dd19";
const GENERATED_AT: &str = "2026-07-25T23:55:00-07:00";
const PRODUCER: &str =
    "G08ScenarioRunner -> ImModule::handle -> Effect::Emit -> InteractionFrameV2::decode";

/// 绑定稳定 fixture id、文件名与生产回放得到的完整 Frame。
struct FixtureCase {
    fixture_id: &'static str,
    file_name: &'static str,
    frame: InteractionFrameV2,
}

/// 从唯一 G08 ScenarioRunner 生成前置、初态、权威终态和恢复态四份完整 Frame。
fn produce_cases() -> Vec<FixtureCase> {
    let created = G08ScenarioRunner::run_created();
    let failed = G08ScenarioRunner::run_http_failure();
    let replayed = G08ScenarioRunner::run_persist_failure_replay();
    let recovered = G08ScenarioRunner::run_recovery();
    assert_eq!(created.group_created, failed.group_created);
    assert_eq!(created.initial_summary, failed.initial_summary);
    assert_eq!(
        created.scheduled_summary,
        replayed.scheduled_after_replay,
        "persist replay must converge to the same terminal Frame"
    );
    assert_group_created_contract_for(&created.group_created, &g08_scenario_spec());
    assert_schedule_summary_contract(&created.initial_summary, false, None);
    assert_schedule_summary_contract(&created.scheduled_summary, true, Some(REQUEST_ID));
    assert_schedule_summary_contract(&recovered.recovered_summary, true, None);
    vec![
        FixtureCase {
            fixture_id: "g08-group-created-v2",
            file_name: "group-created.frame-v2.json",
            frame: created.group_created,
        },
        FixtureCase {
            fixture_id: "g08-channel-summary-empty-v2",
            file_name: "channel-summary-empty.frame-v2.json",
            frame: created.initial_summary,
        },
        FixtureCase {
            fixture_id: "g08-channel-summary-scheduled-v2",
            file_name: "channel-summary-scheduled.frame-v2.json",
            frame: created.scheduled_summary,
        },
        FixtureCase {
            fixture_id: "g08-channel-summary-recovery-v2",
            file_name: "channel-summary-recovery.frame-v2.json",
            frame: recovered.recovered_summary,
        },
    ]
}

/// 生成稳定 pretty JSON bytes，并固定末尾换行。
fn canonical_bytes(frame: &InteractionFrameV2) -> Vec<u8> {
    let mut bytes = serde_json::to_vec_pretty(frame).expect("G08 Frame serializes");
    bytes.push(b'\n');
    bytes
}

/// 提取排除 transaction/revision rebase 后仍须跨端一致的 G08 业务字段。
fn semantic_bytes(frame: &InteractionFrameV2) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "schemaVersion": frame.schema_version,
        "runtime": frame.runtime,
        "surfaces": surface_names(frame),
        "summary": schedule_contract(frame),
    }))
    .expect("G08 semantic projection serializes")
}

/// 提取 channel-summary 的稳定 schedule 字段与 action 能力。
fn schedule_contract(frame: &InteractionFrameV2) -> Option<Value> {
    let view = frame
        .mutations
        .iter()
        .find_map(|mutation| match &mutation.mutation.mutation {
            TypedSurfaceMutation::ChannelSummarySnapshot { view } => Some(view),
            TypedSurfaceMutation::ChannelSummaryPatch { patch } => Some(&patch.view),
            _ => None,
        })?;
    Some(json!({
        "channelId": view.channel_id,
        "hasScheduledMessage": view.has_scheduled_message,
        "scheduleId": view.schedule_id,
        "scheduleOwnerUserId": view.schedule_owner_user_id,
        "scheduleMessage": view.schedule_message,
        "scheduleMessagePreview": view.schedule_message_preview,
        "scheduleSendAt": view.schedule_send_at,
        "scheduleStatus": view.schedule_status,
        "scheduleRevision": view.schedule_revision,
        "actions": view.actions.iter().map(|action| json!({
            "kind": action.kind,
            "enabled": action.enabled,
            "disabledReason": action.disabled_reason,
        })).collect::<Vec<_>>(),
    }))
}

/// 返回 Helix canonical G08 Fixture 目录。
fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("g08-wdio")
}

/// 为一份 G08 Fixture 生成 producer、hash、surface 与动态字段白名单。
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
            "/mutations/*/mutation/view/actions/*/surfaceEpoch",
            "/mutations/*/mutation/view/sections/*/items/*/channelId",
            "/mutations/*/mutation/view/sections/*/items/*/lastActivityAt",
            "/mutations/*/mutation/view/sections/*/items/*/title/text",
            "/mutations/*/mutation/view/sections/*/items/*/avatar/userIds/*",
            "/mutations/*/mutation/view/sections/*/items/*/actions/*/surfaceEpoch",
            "/mutations/*/mutation/patch/view/channelId",
            "/mutations/*/mutation/patch/view/actions/*/surfaceEpoch",
            "/mutations/*/mutation/patch/view/sections/*/items/*/channelId",
            "/mutations/*/mutation/patch/view/sections/*/items/*/lastActivityAt",
            "/mutations/*/mutation/patch/view/sections/*/items/*/title/text",
            "/mutations/*/mutation/patch/view/sections/*/items/*/avatar/userIds/*",
            "/mutations/*/mutation/patch/view/sections/*/items/*/actions/*/surfaceEpoch"
        ],
        "rebaseFields": [
            "/transactionId",
            "/mutations/*/surfaceEpoch",
            "/mutations/*/revision",
            "/mutations/*/baseRevision",
            "/mutations/*/mutation/view/actions/*/surfaceEpoch",
            "/mutations/*/mutation/patch/view/actions/*/surfaceEpoch"
        ],
        "placeholderValues": {
            "actorId": ACTOR_ID,
            "observerId": OBSERVER_ID,
            "teamId": TEAM_ID,
            "channelId": CHANNEL_ID,
            "groupName": GROUP_NAME,
            "createdAt": CREATED_AT,
            "marker": MARKER,
            "reqId": case.frame.causation_id.as_deref().unwrap_or(REQUEST_ID),
            "transactionId": case.frame.transaction_id,
            "surfaceEpoch": case.frame.mutations[0].surface_epoch,
            "revision": case.frame.mutations[0].mutation.revision,
            "baseRevision": case.frame.mutations[0].mutation.base_revision
        }
    })
}

#[test]
/// 证明 actual producer、字段语义、Golden bytes 与 provenance 始终同源。
fn current_g08_producer_matches_canonical_v2_fixtures_and_provenance() {
    let cases = produce_cases();
    assert_eq!(cases.len(), 4, "G08 canonical inventory is explicit");
    let directory = fixture_dir();
    let update = std::env::var_os("UPDATE_G08_WDIO_FIXTURES").is_some();
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
        serde_json::to_vec_pretty(&provenance).expect("G08 provenance serializes");
    provenance_bytes.push(b'\n');
    if update {
        fs::write(directory.join("provenance.json"), &provenance_bytes)
            .expect("provenance export succeeds");
    }
    for case in &cases {
        let path = directory.join(case.file_name);
        let checked_in = fs::read(&path).unwrap_or_else(|error| {
            panic!(
                "{} missing ({error}); run UPDATE_G08_WDIO_FIXTURES=1 cargo test -p helix-im --test g08_wdio_fixture_contract_test",
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
        "G08 provenance hashes or producer identity drifted"
    );
}
