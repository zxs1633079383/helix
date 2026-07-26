//! G10a WDIO/Mobile Frame V2 canonical Fixture 真源。
//!
//! 普通测试只比较真实 producer 与 checked-in Golden；仅
//! `UPDATE_G10A_WDIO_FIXTURES=1` 允许覆盖 canonical JSON。

use std::fs;
use std::path::{Path, PathBuf};

use helix_im::interaction_frame::{InteractionFrameV2, SurfaceLoadState, TypedSurfaceMutation};
use serde_json::{json, Value};

#[path = "support/g10a_scenario.rs"]
#[allow(dead_code)]
mod scenario;

use scenario::{
    assert_group_created_contract_for, assert_timeline_contract, g10a_scenario_spec, sha256_hex,
    surface_names, G10AScenarioRunner, ACTOR_ID, CHANNEL_ID, CREATED_AT, GROUP_NAME, OBSERVER_ID,
    TEAM_ID, WINDOW_TOKEN,
};

const HELIX_COMMIT: &str = "d2b1bf6c1304155d6897a7238cfaefd2bae3d176";
const GENERATED_AT: &str = "2026-07-25T23:15:00-07:00";
const PRODUCER: &str =
    "G10AScenarioRunner -> ImModule::handle -> Effect::Emit -> InteractionFrameV2::decode";

/// 绑定稳定 fixture id、文件名与生产回放得到的完整 Frame。
struct FixtureCase {
    fixture_id: &'static str,
    file_name: &'static str,
    frame: InteractionFrameV2,
}

/// 从唯一 G10a ScenarioRunner 生成前置、成功、失败、竞态与耗尽六份完整 Frame。
fn produce_cases() -> Vec<FixtureCase> {
    let success = G10AScenarioRunner::run_success();
    let http_failure = G10AScenarioRunner::run_http_failure();
    let persist_failure = G10AScenarioRunner::run_persist_failure();
    let race = G10AScenarioRunner::run_duplicate_replies();
    let recovery = G10AScenarioRunner::run_exhaustion_recovery();
    assert_eq!(success.group_created, http_failure.group_created);
    assert_eq!(success.group_created, persist_failure.group_created);
    assert_group_created_contract_for(&success.group_created, &g10a_scenario_spec());
    assert_timeline_contract(&success.loaded, SurfaceLoadState::Ready, 40, true);
    assert_timeline_contract(&http_failure.failed, SurfaceLoadState::Failed, 20, true);
    assert_timeline_contract(&persist_failure.failed, SurfaceLoadState::Failed, 20, true);
    assert_timeline_contract(&race.loaded, SurfaceLoadState::Ready, 40, true);
    assert_timeline_contract(&recovery.exhausted, SurfaceLoadState::Ready, 40, false);
    vec![
        FixtureCase {
            fixture_id: "g10a-group-created-v2",
            file_name: "group-created.frame-v2.json",
            frame: success.group_created,
        },
        FixtureCase {
            fixture_id: "g10a-timeline-loaded-v2",
            file_name: "timeline-loaded.frame-v2.json",
            frame: success.loaded,
        },
        FixtureCase {
            fixture_id: "g10a-timeline-http-failed-v2",
            file_name: "timeline-http-failed.frame-v2.json",
            frame: http_failure.failed,
        },
        FixtureCase {
            fixture_id: "g10a-timeline-persist-failed-v2",
            file_name: "timeline-persist-failed.frame-v2.json",
            frame: persist_failure.failed,
        },
        FixtureCase {
            fixture_id: "g10a-timeline-race-current-v2",
            file_name: "timeline-race-current.frame-v2.json",
            frame: race.loaded,
        },
        FixtureCase {
            fixture_id: "g10a-timeline-exhausted-v2",
            file_name: "timeline-exhausted.frame-v2.json",
            frame: recovery.exhausted,
        },
    ]
}

/// 生成稳定 pretty JSON bytes，并固定末尾换行。
fn canonical_bytes(frame: &InteractionFrameV2) -> Vec<u8> {
    let mut bytes = serde_json::to_vec_pretty(frame).expect("G10a Frame serializes");
    bytes.push(b'\n');
    bytes
}

/// 提取排除运行期 identity 后仍须跨端一致的 G10a 业务字段。
fn semantic_bytes(frame: &InteractionFrameV2) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "schemaVersion": frame.schema_version,
        "runtime": frame.runtime,
        "surfaces": surface_names(frame),
        "timeline": timeline_contract(frame),
    }))
    .expect("G10a semantic projection serializes")
}

/// 提取 timeline 的稳定 state、page、content、delivery 与 action 能力。
fn timeline_contract(frame: &InteractionFrameV2) -> Option<Value> {
    let view = frame
        .mutations
        .iter()
        .find_map(|mutation| match &mutation.mutation.mutation {
            TypedSurfaceMutation::TimelineSnapshot { view } => Some(view),
            TypedSurfaceMutation::TimelinePatch { patch } => Some(&patch.view),
            _ => None,
        })?;
    Some(json!({
        "channelId": view.channel_id,
        "anchorMode": view.anchor.mode,
        "itemCount": view.items.len(),
        "items": view.items.iter().map(|item| json!({
            "channelId": item.channel_id,
            "author": item.author,
            "content": item.content,
            "delivery": item.delivery,
            "actions": item.actions.iter().map(|action| json!({
                "kind": action.kind,
                "inputShape": action.input_shape,
                "enabled": action.enabled,
                "disabledReason": action.disabled_reason,
            })).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
        "page": view.page,
        "state": view.state,
    }))
}

/// 返回 Helix canonical G10a Fixture 目录。
fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("g10a-wdio")
}

/// 为一份 G10a Fixture 生成 producer、hash、surface 与动态字段白名单。
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
            "/mutations/*/scope/windowToken",
            "/mutations/*/scope/userId",
            "/mutations/*/scope/teamId",
            "/mutations/*/mutation/view/channelId",
            "/mutations/*/mutation/view/page/windowToken",
            "/mutations/*/mutation/view/items/*/id",
            "/mutations/*/mutation/view/items/*/channelId",
            "/mutations/*/mutation/view/items/*/author/userId",
            "/mutations/*/mutation/view/items/*/createdAt",
            "/mutations/*/mutation/view/items/*/content/text",
            "/mutations/*/mutation/view/items/*/actions/*/actionRef",
            "/mutations/*/mutation/view/items/*/actions/*/surfaceEpoch",
            "/mutations/*/mutation/view/sections/*/items/*/channelId",
            "/mutations/*/mutation/view/sections/*/items/*/lastActivityAt",
            "/mutations/*/mutation/view/sections/*/items/*/title/text",
            "/mutations/*/mutation/view/sections/*/items/*/avatar/userIds/*",
            "/mutations/*/mutation/view/sections/*/items/*/avatar/fallbackText",
            "/mutations/*/mutation/patch/view/channelId",
            "/mutations/*/mutation/patch/view/page/windowToken",
            "/mutations/*/mutation/patch/view/items/*/id",
            "/mutations/*/mutation/patch/view/items/*/channelId",
            "/mutations/*/mutation/patch/view/items/*/author/userId",
            "/mutations/*/mutation/patch/view/items/*/createdAt",
            "/mutations/*/mutation/patch/view/items/*/content/text",
            "/mutations/*/mutation/patch/view/items/*/actions/*/actionRef",
            "/mutations/*/mutation/patch/view/items/*/actions/*/surfaceEpoch",
            "/mutations/*/mutation/patch/view/sections/*/items/*/channelId",
            "/mutations/*/mutation/patch/view/sections/*/items/*/lastActivityAt",
            "/mutations/*/mutation/patch/view/sections/*/items/*/title/text",
            "/mutations/*/mutation/patch/view/sections/*/items/*/avatar/userIds/*",
            "/mutations/*/mutation/patch/view/sections/*/items/*/avatar/fallbackText"
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
            "channelId": CHANNEL_ID,
            "windowToken": WINDOW_TOKEN,
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
fn current_g10a_producer_matches_canonical_v2_fixtures_and_provenance() {
    let cases = produce_cases();
    assert_eq!(cases.len(), 6, "G10a canonical inventory is explicit");
    let directory = fixture_dir();
    let update = std::env::var_os("UPDATE_G10A_WDIO_FIXTURES").is_some();
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
        serde_json::to_vec_pretty(&provenance).expect("G10a provenance serializes");
    provenance_bytes.push(b'\n');
    if update {
        fs::write(directory.join("provenance.json"), &provenance_bytes)
            .expect("provenance export succeeds");
    }
    for case in &cases {
        let path = directory.join(case.file_name);
        let checked_in = fs::read(&path).unwrap_or_else(|error| {
            panic!(
                "{} missing ({error}); run UPDATE_G10A_WDIO_FIXTURES=1 cargo test -p helix-im --test g10a_wdio_fixture_contract_test",
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
        "G10a provenance hashes or producer identity drifted"
    );
}
