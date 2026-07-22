use std::collections::{BTreeMap, BTreeSet};

use helix_im::interaction_frame::{InteractionFrame, INTERACTION_FRAME_SCHEMA_VERSION};
use serde::Deserialize;

const MANIFEST: &str = include_str!("fixtures/uc01-16-surface-runtime/manifest.json");
const SNAPSHOT: &str = include_str!("fixtures/uc01-16-surface-runtime/snapshots.json");
const PATCH: &str = include_str!("fixtures/uc01-16-surface-runtime/patch-transaction.json");
const DUPLICATE: &str = include_str!("fixtures/uc01-16-surface-runtime/duplicate.json");
const STALE: &str = include_str!("fixtures/uc01-16-surface-runtime/stale.json");
const GAP: &str = include_str!("fixtures/uc01-16-surface-runtime/gap.json");
const SCOPE_MISMATCH: &str = include_str!("fixtures/uc01-16-surface-runtime/scope-mismatch.json");

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FixtureManifest {
    fixture_manifest_version: u32,
    interaction_frame_schema_version: u32,
    source_revisions: BTreeMap<String, String>,
    fixtures: BTreeMap<String, String>,
    producer_status: BTreeMap<String, String>,
    v2_migration: V2Migration,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct V2Migration {
    source_pin: String,
    target_schema_version: u32,
    required_fixtures: BTreeMap<String, String>,
    consumer_rule: String,
}

#[test]
fn manifest_pins_the_contract_foundation_and_live_producer_revision() {
    let manifest: FixtureManifest = serde_json::from_str(MANIFEST).expect("fixture manifest JSON");
    assert_eq!(manifest.fixture_manifest_version, 1);
    assert_eq!(
        manifest.interaction_frame_schema_version,
        INTERACTION_FRAME_SCHEMA_VERSION
    );
    assert_eq!(
        manifest
            .source_revisions
            .get("interactionFrameFoundation")
            .map(String::as_str),
        Some("756d853")
    );
    assert_eq!(
        manifest
            .source_revisions
            .get("dialogUnreadTimelineLiveProducer")
            .map(String::as_str),
        Some("51111912a164d95bd0a025a39e30b879fd935fd5")
    );
    assert_eq!(
        manifest.fixtures.values().cloned().collect::<BTreeSet<_>>(),
        BTreeSet::from([
            "accepted-action.json".to_string(),
            "duplicate.json".to_string(),
            "gap.json".to_string(),
            "patch-transaction.json".to_string(),
            "rejected-action.json".to_string(),
            "rich-v2.json".to_string(),
            "scope-mismatch.json".to_string(),
            "snapshots.json".to_string(),
            "stale.json".to_string(),
        ])
    );
}

#[test]
fn manifest_lists_all_eight_v1_surfaces_with_their_actual_live_producer_status() {
    let manifest: FixtureManifest = serde_json::from_str(MANIFEST).expect("fixture manifest JSON");
    assert_eq!(manifest.producer_status.len(), 8);
    for surface in [
        "message.thread-detail",
        "message.participant-receipts",
        "message.channel-summary",
        "message.member-roster",
        "message.topic-list",
    ] {
        assert!(
            manifest
                .producer_status
                .get(surface)
                .is_some_and(|status| status.starts_with("live:")),
            "{surface} must name its actual V1 producer status"
        );
    }
}

#[test]
fn v2_migration_reserves_all_gs01_gs04_fixtures_without_claiming_v1_is_v2() {
    let manifest: FixtureManifest = serde_json::from_str(MANIFEST).expect("fixture manifest JSON");
    assert_eq!(manifest.v2_migration.source_pin, "b4339e252e865ea294c51001388b1ef0de3ba42e");
    assert_eq!(manifest.v2_migration.target_schema_version, 2);
    assert_eq!(manifest.v2_migration.required_fixtures.len(), 7);
    assert!(manifest
        .v2_migration
        .required_fixtures
        .values()
        .all(|name| name.ends_with("-v2.json")));
    assert!(manifest.v2_migration.consumer_rule.contains("never a fallback"));
}

#[test]
fn every_frame_fixture_declares_the_pinned_schema_version() {
    for fixture in [SNAPSHOT, PATCH, DUPLICATE, STALE, GAP, SCOPE_MISMATCH] {
        let value: serde_json::Value = serde_json::from_str(fixture).expect("fixture JSON");
        assert_eq!(
            value["schemaVersion"].as_u64(),
            Some(u64::from(INTERACTION_FRAME_SCHEMA_VERSION))
        );
    }
    InteractionFrame::decode(SNAPSHOT.as_bytes()).expect("canonical snapshot remains decodable");
    InteractionFrame::decode(PATCH.as_bytes()).expect("canonical patch remains decodable");
}
