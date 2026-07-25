use bytes::Bytes;
use helix_core::effect::Effect;
use serde_json::json;

use crate::support::*;

#[test]
fn http_success_releases_sent_frame_only_after_terminal_persist_readback() {
    let mut replay = Replay::new();
    attach_timeline(&mut replay);
    let sending = start_sending(&mut replay);

    replay.step(
        ok(
            sending.http_corr,
            http_envelope(json!({
                "status": "SUCCESS",
                "data": {"id": SERVER_ID}
            })),
        ),
        CREATED_AT as u64 + 13,
    );
    assert!(
        replay
            .effects()
            .iter()
            .any(|effect| matches!(effect, Effect::CancelTimer { id } if *id == sending.timer_id)),
        "HTTP terminal authority cancels the timeout"
    );
    assert!(
        replay.frames().is_empty(),
        "HTTP success is not itself a render-ready sent Frame"
    );
    let terminal_corr = replay.only_persist_corr_with(|ops| has_table_write(ops, "message"));

    replay.step(ok(terminal_corr, Bytes::new()), CREATED_AT as u64 + 14);
    assert!(
        replay.frames().is_empty(),
        "terminal PersistOk schedules readback; it does not fabricate a row"
    );
    let read_corr = replay.only_persist_corr_with(|ops| {
        ops.iter().any(
            |op| matches!(op, helix_core::effect::StorageOp::Scan(spec) if spec.table == "message"),
        )
    });
    replay.step(
        ok(
            read_corr,
            Bytes::from(
                serde_json::to_vec(&json!([message_row(
                    SERVER_ID,
                    &sending.temporary_id,
                    "sent"
                )]))
                .expect("sent row serializes"),
            ),
        ),
        CREATED_AT as u64 + 15,
    );
    assert_delivery(&replay, "sent", SERVER_ID);
}

#[test]
fn http_error_releases_failed_frame_only_after_unsend_readback() {
    let mut replay = Replay::new();
    attach_timeline(&mut replay);
    let sending = start_sending(&mut replay);

    replay.step(http_error(sending.http_corr), CREATED_AT as u64 + 13);
    assert!(
        replay.frames().is_empty(),
        "transport failure is not rendered before the durable unsend fact"
    );
    let failed_corr = replay.only_persist_corr_with(|ops| has_table_write(ops, "message"));
    replay.step(ok(failed_corr, Bytes::new()), CREATED_AT as u64 + 14);
    let read_corr = replay.only_persist_corr_with(|ops| {
        ops.iter().any(
            |op| matches!(op, helix_core::effect::StorageOp::Scan(spec) if spec.table == "message"),
        )
    });
    replay.step(
        ok(
            read_corr,
            Bytes::from(
                serde_json::to_vec(&json!([message_row(
                    &sending.temporary_id,
                    &sending.temporary_id,
                    "unsend"
                )]))
                .expect("failed row serializes"),
            ),
        ),
        CREATED_AT as u64 + 15,
    );
    assert_delivery(&replay, "failed", &sending.temporary_id);
}
