//! G-15 WDIO/Mobile Frame V2 canonical Fixture 真源。
//!
//! 所有 JSON 来自 `G15ScenarioRunner -> ImModule::handle -> Effect::Emit`；普通测试只比较
//! actual 与 Golden，只有 `UPDATE_G15_WDIO_FIXTURES=1` 才允许覆盖。

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use helix_im::interaction_frame::InteractionFrameV2;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

#[path = "support/g15_scenario.rs"]
mod scenario;

use scenario::{
    assert_summary_contract, assert_thread_contract, G15ScenarioRunner, ACTOR_ID, OBSERVER_ID,
    PARENT_CHANNEL_ID, REQUEST_ID, ROOT_MESSAGE_ID, TEAM_ID, TOPIC_CHANNEL_ID, TOPIC_TITLE,
};

const HELIX_COMMIT: &str = "064f814a0f4fc4df8b9299652d0e615f5857b875";
const GENERATED_AT: &str = "2026-07-26T12:03:17-07:00";
const PRODUCER: &str =
    "G15ScenarioRunner -> ImModule::handle -> Effect::Emit -> InteractionFrameV2::decode";

/// 描述一份 G-15 canonical Fixture 的稳定 identity 和 producer 输出。
struct FixtureCase {
    fixture_id: &'static str,
    file_name: &'static str,
    frame: InteractionFrameV2,
}

/// 从唯一生产 runner 生成 terminal thread 与 durable summary recovery Frame。
fn produce_cases() -> Vec<FixtureCase> {
    let created = G15ScenarioRunner::run_http_created();
    let recovered = G15ScenarioRunner::run_recovery();
    let ws_created = G15ScenarioRunner::run_ws_created();
    assert_thread_contract(&created.thread);
    assert_summary_contract(&recovered.summary);
    let dialog = ws_frame_for(&ws_created.frames, "message.left-dialog-list");
    let channel = ws_frame_for(&ws_created.frames, "message.channel-summary");
    vec![
        FixtureCase {
            fixture_id: "g15-topic-thread-created-v2",
            file_name: "topic-thread-created.frame-v2.json",
            frame: created.thread,
        },
        FixtureCase {
            fixture_id: "g15-channel-summary-recovery-v2",
            file_name: "channel-summary-recovery.frame-v2.json",
            frame: recovered.summary,
        },
        FixtureCase {
            fixture_id: "g15-ws-topic-dialog-v2",
            file_name: "ws-topic-dialog.frame-v2.json",
            frame: dialog,
        },
        FixtureCase {
            fixture_id: "g15-ws-topic-created-v2",
            file_name: "ws-topic-created.frame-v2.json",
            frame: channel,
        },
    ]
}

/// 从独立 WS terminal 批次提取包含目标 surface 的唯一完整 Frame。
fn ws_frame_for(frames: &[InteractionFrameV2], surface: &str) -> InteractionFrameV2 {
    let mut matching = frames.iter().filter(|frame| {
        frame
            .mutations
            .iter()
            .any(|mutation| mutation.mutation.surface.as_str() == surface)
    });
    let frame = matching.next().expect("G15 WS target Frame").clone();
    assert!(matching.next().is_none());
    frame
}

/// 生成稳定 pretty JSON bytes 并固定末尾换行。
fn canonical_bytes(frame: &InteractionFrameV2) -> Vec<u8> {
    let mut bytes = serde_json::to_vec_pretty(frame).expect("G15 Frame serializes");
    bytes.push(b'\n');
    bytes
}

/// 提取排除 transaction/revision 后仍必须跨端一致的业务语义。
fn semantic_bytes(frame: &InteractionFrameV2) -> Vec<u8> {
    let views = frame
        .mutations
        .iter()
        .map(|mutation| {
            json!({
                "surface": mutation.mutation.surface,
                "scope": mutation.mutation.scope,
                "mutation": mutation.mutation.mutation,
            })
        })
        .collect::<Vec<_>>();
    serde_json::to_vec(&json!({
        "schemaVersion": frame.schema_version,
        "runtime": frame.runtime,
        "surfaces": surface_names(frame),
        "views": views,
    }))
    .expect("G15 semantic projection serializes")
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

/// 返回 Helix canonical G-15 Fixture 目录。
fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("g15-wdio")
}

/// 为一份 Fixture 生成真源、hash、surface 与唯一允许的动态字段白名单。
fn provenance_entry(case: &FixtureCase, bytes: &[u8]) -> Value {
    json!({
        "fixtureId": case.fixture_id,
        "fileName": case.file_name,
        "gateId": "G-15",
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
            "/mutations/*/mutation/view/channelId",
            "/mutations/*/mutation/view/detailContext/parentChannelId",
            "/mutations/*/mutation/view/detailContext/topicChannelId",
            "/mutations/*/mutation/view/detailContext/rootMessageId",
            "/mutations/*/mutation/view/detailContext/title",
            "/mutations/*/mutation/view/root/id",
            "/mutations/*/mutation/view/root/channelId",
            "/mutations/*/mutation/view/root/author/userId",
            "/mutations/*/mutation/view/title/text",
            "/mutations/*/mutation/view/actions/*/surfaceEpoch"
        ],
        "rebaseFields": [
            "/transactionId",
            "/mutations/*/surfaceEpoch",
            "/mutations/*/revision",
            "/mutations/*/baseRevision",
            "/mutations/*/mutation/view/root/actions/*/surfaceEpoch",
            "/mutations/*/mutation/view/actions/*/surfaceEpoch"
        ],
        "placeholderValues": {
            "actorId": ACTOR_ID,
            "observerId": OBSERVER_ID,
            "teamId": TEAM_ID,
            "parentChannelId": PARENT_CHANNEL_ID,
            "channelId": TOPIC_CHANNEL_ID,
            "rootMessageId": ROOT_MESSAGE_ID,
            "topicTitle": TOPIC_TITLE,
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
fn current_g15_producer_matches_canonical_v2_fixtures_and_provenance() {
    let cases = produce_cases();
    assert_eq!(cases.len(), 4, "G15 canonical inventory is explicit");
    assert_eq!(
        cases
            .iter()
            .flat_map(|case| surface_names(&case.frame))
            .collect::<BTreeSet<_>>(),
        BTreeSet::from([
            "message.channel-summary",
            "message.left-dialog-list",
            "message.member-roster",
            "message.thread-detail",
        ])
    );
    let directory = fixture_dir();
    let update = std::env::var_os("UPDATE_G15_WDIO_FIXTURES").is_some();
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
        serde_json::to_vec_pretty(&provenance).expect("G15 provenance serializes");
    provenance_bytes.push(b'\n');
    if update {
        fs::write(directory.join("provenance.json"), &provenance_bytes)
            .expect("provenance export succeeds");
    }
    for case in &cases {
        let path = directory.join(case.file_name);
        let checked_in = fs::read(&path).unwrap_or_else(|error| {
            panic!(
                "{} missing ({error}); run UPDATE_G15_WDIO_FIXTURES=1 cargo test -p helix-im --test g15_wdio_fixture_contract_test",
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
        "G15 provenance hashes or producer identity drifted"
    );
}
