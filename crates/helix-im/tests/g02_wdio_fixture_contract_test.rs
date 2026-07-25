//! G02 WDIO Frame V2 canonical Fixture 真源。
//!
//! 所有文件都来自共享 `G02ScenarioRunner` 的公开 `ImModule::handle` 回放；普通测试只比较
//! actual Frame 与 checked-in Golden，只有显式设置 `UPDATE_G02_WDIO_FIXTURES=1` 才写盘。

use std::fs;
use std::path::{Path, PathBuf};

use helix_im::interaction_frame::{
    InteractionFrameV2, MessageContentVm, TypedSurfaceMutation,
};
use serde_json::{json, Value};

#[path = "support/g02_scenario.rs"]
mod scenario;

use scenario::{
    assert_dialog_contract_for, assert_document_timeline_contract,
    assert_group_created_contract_for, delivery_kinds, first_message, g02_scenario_spec,
    surface_names, G02ScenarioRunner, Terminal, ACTOR_ID, CHANNEL_ID, CREATED_AT, DOCUMENT_URL,
    GROUP_NAME, MARKER, OBSERVER_ID, REQUEST_ID, SERVER_ID, TEAM_ID,
};

const HELIX_COMMIT: &str = "ac15abd0b29ef62ea5d359fb22030800dd0dee2b";
const GENERATED_AT: &str = "2026-07-25T09:00:00-07:00";
const PRODUCER: &str =
    "G02ScenarioRunner -> ImModule::handle -> Effect::Emit -> InteractionFrameV2::decode";

/// 描述一个 canonical Fixture 的稳定 ID、文件名和 producer 输出。
struct FixtureCase {
    fixture_id: &'static str,
    file_name: &'static str,
    frame: InteractionFrameV2,
}

/// 从唯一 ScenarioRunner 生成建群前置及 HTTP/WS 两条 correlation 的六份 Frame。
fn produce_cases() -> Vec<FixtureCase> {
    let sent = G02ScenarioRunner::run_http(Terminal::Sent);
    let business_failed = G02ScenarioRunner::run_http(Terminal::BusinessFailed);
    let failed = G02ScenarioRunner::run_http(Terminal::Failed);
    let ws = G02ScenarioRunner::run_ws_echo_with_dialog();
    assert_eq!(
        sent.sending, failed.sending,
        "sending Frame cannot depend on its future HTTP terminal"
    );
    assert_eq!(
        sent.group_created, failed.group_created,
        "group prerequisite cannot depend on its future message terminal"
    );
    assert_eq!(
        business_failed.terminal, failed.terminal,
        "business timeout and infrastructure error must converge on one failed Frame contract"
    );
    assert_eq!(
        sent.group_created, ws.group_created,
        "HTTP and WS message branches must start from the same created group"
    );
    assert_eq!(
        sent.sending, ws.sending,
        "dialog attachment cannot mutate the independent timeline sending Frame"
    );
    assert_eq!(
        sent.temporary_id, ws.temporary_id,
        "HTTP and WS branches must correlate the same optimistic message"
    );
    assert_ne!(
        ws.timeline_corr, ws.dialog_corr,
        "WS timeline and dialog must keep independent persist barriers"
    );
    let spec = g02_scenario_spec();
    assert_group_created_contract_for(&sent.group_created, &spec);
    assert_document_timeline_contract(&sent.sending, "sending", &sent.temporary_id);
    assert_document_timeline_contract(&sent.terminal, "sent", SERVER_ID);
    assert_document_timeline_contract(&failed.terminal, "failed", &failed.temporary_id);
    assert_document_timeline_contract(&ws.timeline_sent, "sent", SERVER_ID);
    assert_eq!(
        first_message(&ws.timeline_sent).event_seq,
        1,
        "WS timeline Frame must preserve the authoritative event_seq"
    );
    assert_dialog_contract_for(&ws.dialog, &spec);
    vec![
        FixtureCase {
            fixture_id: "g02-group-created-v2",
            file_name: "group-created.frame-v2.json",
            frame: sent.group_created,
        },
        FixtureCase {
            fixture_id: "g02-sending-v2",
            file_name: "sending.frame-v2.json",
            frame: sent.sending,
        },
        FixtureCase {
            fixture_id: "g02-sent-v2",
            file_name: "sent.frame-v2.json",
            frame: sent.terminal,
        },
        FixtureCase {
            fixture_id: "g02-failed-v2",
            file_name: "failed.frame-v2.json",
            frame: failed.terminal,
        },
        FixtureCase {
            fixture_id: "g02-ws-timeline-sent-v2",
            file_name: "ws-timeline-sent.frame-v2.json",
            frame: ws.timeline_sent,
        },
        FixtureCase {
            fixture_id: "g02-ws-left-dialog-list-v2",
            file_name: "ws-left-dialog-list.frame-v2.json",
            frame: ws.dialog,
        },
    ]
}

/// 生成仓库稳定的 pretty JSON bytes，并固定末尾换行。
fn canonical_bytes(frame: &InteractionFrameV2) -> Vec<u8> {
    let mut bytes = serde_json::to_vec_pretty(frame).expect("Frame serializes");
    bytes.push(b'\n');
    bytes
}

/// 生成跨仓 parity 使用的业务语义摘要，排除 transaction 等可参数化身份。
fn semantic_bytes(frame: &InteractionFrameV2) -> Vec<u8> {
    let mutation_kinds: Vec<&str> = frame
        .mutations
        .iter()
        .map(|mutation| match mutation.mutation.mutation {
            TypedSurfaceMutation::TimelineSnapshot { .. } => "timelineSnapshot",
            TypedSurfaceMutation::TimelinePatch { .. } => "timelinePatch",
            TypedSurfaceMutation::DialogListSnapshot { .. } => "dialogListSnapshot",
            TypedSurfaceMutation::DialogListPatch { .. } => "dialogListPatch",
            TypedSurfaceMutation::UnreadSummarySnapshot { .. } => "unreadSummarySnapshot",
            TypedSurfaceMutation::UnreadSummaryPatch { .. } => "unreadSummaryPatch",
            _ => "other",
        })
        .collect();
    serde_json::to_vec(&json!({
        "schemaVersion": frame.schema_version,
        "runtime": frame.runtime,
        "surfaces": surface_names(frame),
        "mutationKinds": mutation_kinds,
        "deliveryKinds": delivery_kinds(frame),
        "documentContract": document_contract(frame),
        "dialogContract": dialog_contract(frame),
    }))
    .expect("semantic projection serializes")
}

/// 提取 G02 固定文档卡片的稳定业务字段，排除可 rebase 的 message identity。
fn document_contract(frame: &InteractionFrameV2) -> Option<Value> {
    let message = frame.mutations.iter().find_map(|mutation| match &mutation.mutation.mutation {
        TypedSurfaceMutation::TimelineSnapshot { view } => view.items.first(),
        TypedSurfaceMutation::TimelinePatch { patch } => patch.view.items.first(),
        _ => None,
    })?;
    let MessageContentVm::Document {
        title,
        summary,
        access,
        ..
    } = &message.content
    else {
        return None;
    };
    Some(json!({
        "kind": "document",
        "title": title,
        "summary": summary,
        "access": access,
    }))
}

/// 提取 dialog preview 的稳定业务形状，区分 fresh-group empty 与 WS document patch。
fn dialog_contract(frame: &InteractionFrameV2) -> Option<Value> {
    let row = frame.mutations.iter().find_map(|mutation| match &mutation.mutation.mutation {
        TypedSurfaceMutation::DialogListSnapshot { view } => view
            .sections
            .iter()
            .flat_map(|section| &section.items)
            .next(),
        TypedSurfaceMutation::DialogListPatch { patch } => patch
            .view
            .sections
            .iter()
            .flat_map(|section| &section.items)
            .next(),
        _ => None,
    })?;
    Some(match &row.preview {
        helix_im::interaction_frame::PreviewVm::Empty => json!({"kind": "empty"}),
        helix_im::interaction_frame::PreviewVm::Document { title, .. } => {
            json!({"kind": "document", "title": title})
        }
        preview => json!({"kind": format!("{preview:?}")}),
    })
}

/// 返回 Helix canonical G02 Fixture 目录。
fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("g02-wdio")
}

/// 返回 Frame 的第一条 timeline messageId；dialog-only Frame 没有消息节点。
fn optional_message_id(frame: &InteractionFrameV2) -> Option<&str> {
    frame
        .mutations
        .iter()
        .any(|mutation| {
            matches!(
                mutation.mutation.mutation,
                TypedSurfaceMutation::TimelineSnapshot { .. }
                    | TypedSurfaceMutation::TimelinePatch { .. }
            )
        })
        .then(|| first_message(frame).id.as_str())
}

/// 为每份 Fixture 生成可审计的 producer、hash、surface 和参数白名单。
fn provenance_entry(case: &FixtureCase, bytes: &[u8], temporary_id: &str) -> Value {
    let transaction_id = case.frame.transaction_id.clone();
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
            "/mutations/*/mutation/view/anchor/messageId",
            "/mutations/*/mutation/view/items/*/id",
            "/mutations/*/mutation/view/items/*/channelId",
            "/mutations/*/mutation/view/items/*/author/userId",
            "/mutations/*/mutation/view/items/*/createdAt",
            "/mutations/*/mutation/patch/view/channelId",
            "/mutations/*/mutation/patch/view/anchor/messageId",
            "/mutations/*/mutation/patch/view/items/*/id",
            "/mutations/*/mutation/patch/view/items/*/channelId",
            "/mutations/*/mutation/patch/view/items/*/author/userId",
            "/mutations/*/mutation/patch/view/items/*/createdAt",
            "/mutations/*/mutation/patch/view/sections/*/items/*/channelId",
            "/mutations/*/mutation/patch/view/sections/*/items/*/lastActivityAt",
            "/mutations/*/mutation/patch/view/sections/*/items/*/title/text",
            "/mutations/*/mutation/patch/view/sections/*/items/*/avatar/userIds/*",
            "/mutations/*/mutation/patch/view/sections/*/items/*/avatar/fallbackText",
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
            "/mutations/*/baseRevision"
        ],
        "placeholderValues": {
            "actorId": ACTOR_ID,
            "observerId": OBSERVER_ID,
            "teamId": TEAM_ID,
            "channelId": CHANNEL_ID,
            "groupName": GROUP_NAME,
            "createdAt": CREATED_AT,
            "marker": MARKER,
            "documentUrl": DOCUMENT_URL,
            "temporaryId": temporary_id,
            "postId": SERVER_ID,
            "reqId": REQUEST_ID,
            "transactionId": transaction_id,
            "messageId": optional_message_id(&case.frame),
            "surfaceEpoch": case.frame.mutations[0].surface_epoch,
            "revision": case.frame.mutations[0].mutation.revision,
            "baseRevision": case.frame.mutations[0].mutation.base_revision
        }
    })
}

#[test]
/// 证明 producer actual、字段语义、Golden bytes 与 provenance 始终为同一份合同。
fn current_g02_producer_matches_canonical_v2_fixtures_and_provenance() {
    let cases = produce_cases();
    assert_eq!(cases.len(), 6, "G02 canonical inventory is explicit");
    let temporary_id = first_message(&cases[1].frame).id.clone();
    let directory = fixture_dir();
    let update = std::env::var_os("UPDATE_G02_WDIO_FIXTURES").is_some();
    let mut provenance = Vec::with_capacity(cases.len());

    for case in &cases {
        case.frame.validate().expect("producer Frame remains valid");
        assert_eq!(case.frame.schema_version, 2);
        assert!(!surface_names(&case.frame).is_empty());
        let bytes = canonical_bytes(&case.frame);
        provenance.push(provenance_entry(case, &bytes, &temporary_id));
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
                "{} missing ({error}); run UPDATE_G02_WDIO_FIXTURES=1 cargo test -p helix-im --test g02_wdio_fixture_contract_test",
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
