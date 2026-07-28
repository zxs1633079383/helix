//! G03 WDIO canonical InteractionFrame V2 fixtures.
//!
//! Frames are produced by the real timeline Surface projector. Set
//! `UPDATE_G03_WDIO_FIXTURES=1` explicitly to update checked-in bytes.

use std::fs;
use std::path::{Path, PathBuf};

use helix_im::interaction_frame::{InteractionFrameV2, TypedSurfaceMutation};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

#[path = "support/g03_scenario.rs"]
mod scenario;

use scenario::{assert_g03_frame_contract, G03ScenarioRunner};

const HELIX_COMMIT: &str = "ea3fbf379d359ad2f5ccc0d03976f4341a84d5ae";
const GENERATED_AT: &str = "2026-07-27T16:00:00+08:00";
const PRODUCER: &str =
    "G03ScenarioRunner -> TimelineProjector -> SurfaceMutationV2 -> InteractionFrameV2";

struct FixtureCase {
    fixture_id: &'static str,
    file_name: &'static str,
    case_name: &'static str,
    frame: InteractionFrameV2,
}

fn produce_cases() -> Vec<FixtureCase> {
    let scenario = G03ScenarioRunner::run();
    vec![
        FixtureCase {
            fixture_id: "g03-image-sending-v2",
            file_name: "image-sending.frame-v2.json",
            case_name: "image-sending",
            frame: scenario.image_sending,
        },
        FixtureCase {
            fixture_id: "g03-image-sent-v2",
            file_name: "image-sent.frame-v2.json",
            case_name: "image-sent",
            frame: scenario.image_sent,
        },
        FixtureCase {
            fixture_id: "g03-file-uploading-v2",
            file_name: "file-uploading.frame-v2.json",
            case_name: "file-uploading",
            frame: scenario.file_uploading,
        },
        FixtureCase {
            fixture_id: "g03-file-sent-v2",
            file_name: "file-sent.frame-v2.json",
            case_name: "file-sent",
            frame: scenario.file_sent,
        },
    ]
}

#[test]
fn g03_wdio_fixtures_match_the_surface_frame_v2_producer() {
    let update = std::env::var("UPDATE_G03_WDIO_FIXTURES").as_deref() == Ok("1");
    let fixture_dir = fixture_dir();
    if update {
        fs::create_dir_all(&fixture_dir).expect("create G03 fixture directory");
    }
    let mut provenance = Vec::new();
    for case in produce_cases() {
        assert_g03_frame_contract(&case.frame, case.case_name);
        let bytes = canonical_bytes(&case.frame);
        let path = fixture_dir.join(case.file_name);
        if update {
            fs::write(&path, &bytes).expect("write G03 fixture");
        } else {
            assert_eq!(
                fs::read(&path).expect("read checked-in G03 fixture"),
                bytes,
                "{} drifted from its producer",
                case.file_name
            );
        }
        provenance.push(provenance_entry(&case, &bytes));
    }
    let mut provenance_bytes =
        serde_json::to_vec_pretty(&provenance).expect("provenance serializes");
    provenance_bytes.push(b'\n');
    let provenance_path = fixture_dir.join("provenance.json");
    if update {
        fs::write(provenance_path, provenance_bytes).expect("write G03 provenance");
    } else {
        assert_eq!(
            fs::read(provenance_path).expect("read checked-in G03 provenance"),
            provenance_bytes
        );
    }
}

fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("g03-wdio")
}

fn canonical_bytes(frame: &InteractionFrameV2) -> Vec<u8> {
    let mut bytes = serde_json::to_vec_pretty(frame).expect("Frame serializes");
    bytes.push(b'\n');
    bytes
}

fn provenance_entry(case: &FixtureCase, bytes: &[u8]) -> Value {
    let mutation = &case.frame.mutations[0].mutation;
    let message = match &mutation.mutation {
        TypedSurfaceMutation::TimelineSnapshot { view } => &view.items[0],
        _ => unreachable!("G03 fixtures are timeline snapshots"),
    };
    json!({
        "fileName": case.file_name,
        "fixtureId": case.fixture_id,
        "fixtureSha256": sha256(bytes),
        "generatedAt": GENERATED_AT,
        "helixCommit": HELIX_COMMIT,
        "parameterizableFields": [
            "/transactionId",
            "/causationId",
            "/mutations/*/causationId",
            "/mutations/*/surfaceEpoch",
            "/mutations/*/revision",
            "/mutations/*/baseRevision",
            "/mutations/*/scope/channelId",
            "/mutations/*/mutation/view/channelId",
            "/mutations/*/mutation/view/items/*/id",
            "/mutations/*/mutation/view/items/*/channelId",
            "/mutations/*/mutation/view/items/*/author/userId",
            "/mutations/*/mutation/view/items/*/createdAt"
        ],
        "placeholderValues": {
            "actorId": message.author.user_id,
            "baseRevision": mutation.base_revision,
            "channelId": message.channel_id,
            "createdAt": message.created_at,
            "messageId": message.id,
            "reqId": case.frame.causation_id,
            "revision": mutation.revision,
            "surfaceEpoch": case.frame.mutations[0].surface_epoch,
            "transactionId": case.frame.transaction_id
        },
        "producer": PRODUCER,
        "rebaseFields": [
            "/transactionId",
            "/mutations/*/surfaceEpoch",
            "/mutations/*/revision",
            "/mutations/*/baseRevision"
        ],
        "schemaVersion": 2,
        "semanticSha256": semantic_sha256(&case.frame),
        "sourceKind": "rust-export",
        "surfaces": ["message.timeline"]
    })
}

fn semantic_sha256(frame: &InteractionFrameV2) -> String {
    let message = frame
        .mutations
        .iter()
        .find_map(|mutation| match &mutation.mutation.mutation {
            TypedSurfaceMutation::TimelineSnapshot { view } => view.items.first(),
            _ => None,
        })
        .expect("G03 semantic Frame has one message");
    let value = json!({
        "content": message.content,
        "delivery": message.delivery,
        "runtime": frame.runtime,
        "schemaVersion": frame.schema_version,
        "surfaces": ["message.timeline"]
    });
    sha256(serde_json::to_vec(&value).expect("semantic projection serializes"))
}

fn sha256(bytes: impl AsRef<[u8]>) -> String {
    format!("{:x}", Sha256::digest(bytes.as_ref()))
}
