use bytes::Bytes;
use helix_core::effect::{Effect, StorageOp};

use crate::support::*;

#[test]
/// 证明 WS echo 的 timeline 结算与 dialog patch 使用独立 correlation。
fn ws_post_inbound_reconciles_timeline_and_independently_patches_dialog() {
    let mut replay = Replay::new();
    attach_dialog(&mut replay);
    attach_timeline(&mut replay);
    let sending = start_sending(&mut replay);

    replay.step(echo_frame(&sending.temporary_id), CREATED_AT as u64 + 20);
    assert!(
        replay
            .effects()
            .iter()
            .any(|effect| matches!(effect, Effect::CancelTimer { id } if *id == sending.timer_id)),
        "authoritative post echo must cancel the sending timeout"
    );
    let terminal_corr = replay.only_persist_corr_with(|ops| {
        has_table_write(ops, "message")
            && !ops
                .iter()
                .any(|op| matches!(op, StorageOp::Get(spec) if spec.table == "channel"))
    });
    let dialog_corr = replay.only_persist_corr_with(|ops| {
        ops.iter()
            .any(|op| matches!(op, StorageOp::GuardedBump(spec) if spec.table == "channel"))
            && ops
                .iter()
                .any(|op| matches!(op, StorageOp::Get(spec) if spec.table == "channel"))
    });
    assert_ne!(
        terminal_corr, dialog_corr,
        "timeline reconciliation and dialog projection are independent correlated barriers"
    );

    replay.step(ok(terminal_corr, Bytes::new()), CREATED_AT as u64 + 21);
    let timeline_read_corr = replay.only_persist_corr_with(|ops| {
        ops.iter()
            .any(|op| matches!(op, StorageOp::Scan(spec) if spec.table == "message"))
    });
    replay.step(
        ok(
            timeline_read_corr,
            Bytes::from(
                serde_json::to_vec(&serde_json::json!([message_row(
                    SERVER_ID,
                    &sending.temporary_id,
                    "sent"
                )]))
                .expect("WS terminal row serializes"),
            ),
        ),
        CREATED_AT as u64 + 22,
    );
    assert_delivery(&replay, "sent", SERVER_ID);

    replay.step(
        ok(
            dialog_corr,
            Bytes::from(
                serde_json::to_vec(&serde_json::json!([channel_row(0, TEXT, CREATED_AT + 20)]))
                    .expect("dialog readback serializes"),
            ),
        ),
        CREATED_AT as u64 + 23,
    );
    assert_frame_surface(&replay, "message.left-dialog-list");

    replay.step(
        ok(
            sending.http_corr,
            http_envelope(serde_json::json!({
                "status": "SUCCESS",
                "data": {"id": SERVER_ID}
            })),
        ),
        CREATED_AT as u64 + 24,
    );
    assert!(
        replay
            .effects()
            .iter()
            .all(|effect| !matches!(effect, Effect::Persist { ops, .. } if has_table_write(ops, "message"))),
        "late HTTP success after WS settlement must not create a second terminal write"
    );
}
