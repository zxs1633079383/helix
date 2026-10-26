use super::*;
use crate::state::Seq;
use crate::sync_session::{EventEnvelope, EventKind};
use helix_core::effect::StorageOp;
use helix_core::{Effect, EffectSink};

fn gapped_ev(ch: u64, seq: u64) -> EventEnvelope {
    EventEnvelope::new(
        crate::state::test_channel_id(ch),
        Seq(seq),
        EventKind::PostUpsert,
        crate::sync_session::PostFields::default(),
    )
}

#[test]
fn message_upsert_keeps_dedicated_event_seq_across_historical_refill() {
    let live = event_to_upsert_op(&gapped_ev(1, 7));
    let StorageOp::BatchUpsert(live) = live else {
        panic!("live post must use BatchUpsert");
    };
    assert!(live.rows[0].iter().any(|(column, value)| {
        column == "event_seq" && matches!(value, helix_core::effect::SqlValue::Integer(7))
    }));
    assert!(!live.exclude_from_update.contains(&"event_seq"));

    let historical = event_to_upsert_op(&gapped_ev(1, 0));
    let StorageOp::BatchUpsert(historical) = historical else {
        panic!("historical post must use BatchUpsert");
    };
    assert!(historical.exclude_from_update.contains(&"event_seq"));
}

/// HTTP readback 只覆盖明确提供的富字段，且空 read_bits 不能倒退专用已读回执。
#[test]
fn message_readback_upsert_uses_presence_for_rich_columns() {
    let channel_id = crate::state::test_channel_id(2);
    let mut missing = crate::sync_session::PostFields {
        id: "post-readback".to_string(),
        temporary_id: "tmp-readback".to_string(),
        ..Default::default()
    };
    let missing_event =
        EventEnvelope::new(channel_id, Seq(0), EventKind::PostUpsert, missing.clone());
    let StorageOp::BatchUpsert(missing_spec) = event_to_readback_upsert_op(&missing_event) else {
        panic!("readback post must use BatchUpsert");
    };
    for column in [
        "expedite_map",
        "reply_id",
        "reply_messages",
        "reply_count",
        "read_bits",
    ] {
        assert!(missing_spec.exclude_from_update.contains(&column));
    }

    missing.present_fields = crate::sync_session::POST_FIELD_EXPEDITE_MAP
        | crate::sync_session::POST_FIELD_REPLY_ID
        | crate::sync_session::POST_FIELD_REPLY_MESSAGES
        | crate::sync_session::POST_FIELD_REPLY_COUNT
        | crate::sync_session::POST_FIELD_READ_BITS;
    let explicit_event = EventEnvelope::new(channel_id, Seq(0), EventKind::PostUpsert, missing);
    let StorageOp::BatchUpsert(explicit_spec) = event_to_readback_upsert_op(&explicit_event) else {
        panic!("readback post must use BatchUpsert");
    };
    for column in ["expedite_map", "reply_id", "reply_messages", "reply_count"] {
        assert!(!explicit_spec.exclude_from_update.contains(&column));
    }
    assert!(explicit_spec.exclude_from_update.contains(&"read_bits"));
    assert!(explicit_spec.exclude_from_update.contains(&"event_seq"));

    let mut non_empty = explicit_event.fields.clone();
    non_empty.read_bits = "10".to_string();
    let non_empty_event = EventEnvelope::new(channel_id, Seq(0), EventKind::PostUpsert, non_empty);
    let StorageOp::BatchUpsert(non_empty_spec) = event_to_readback_upsert_op(&non_empty_event)
    else {
        panic!("readback post must use BatchUpsert");
    };
    assert!(!non_empty_spec.exclude_from_update.contains(&"read_bits"));
}

/// Sync 快照缺省或空 read_bits 都不得覆盖已读位；显式清空必须走 type=6 专用写。
#[test]
fn message_sync_upsert_uses_presence_for_read_bits() {
    let channel_id = crate::state::test_channel_id(3);
    let missing = crate::sync_session::PostFields {
        id: "post-sync".to_string(),
        temporary_id: "tmp-sync".to_string(),
        read_bits: String::new(),
        ..Default::default()
    };
    let missing_event = EventEnvelope::new(channel_id, Seq(9), EventKind::PostUpsert, missing);
    let StorageOp::BatchUpsert(missing_spec) = event_to_sync_upsert_op(&missing_event) else {
        panic!("sync post must use BatchUpsert");
    };
    assert!(missing_spec.exclude_from_update.contains(&"read_bits"));

    let mut explicit = crate::sync_session::PostFields {
        id: "post-sync".to_string(),
        temporary_id: "tmp-sync".to_string(),
        read_bits: String::new(),
        ..Default::default()
    };
    explicit.present_fields = crate::sync_session::POST_FIELD_READ_BITS;
    let explicit_event = EventEnvelope::new(channel_id, Seq(9), EventKind::PostUpsert, explicit);
    let StorageOp::BatchUpsert(explicit_spec) = event_to_sync_upsert_op(&explicit_event) else {
        panic!("sync post must use BatchUpsert");
    };
    assert!(explicit_spec.exclude_from_update.contains(&"read_bits"));
}

/// 在线迟到的空 read_bits type=1 只能补消息体，不能擦除更新的 type=6 已读位。
#[test]
fn online_gate_post_upsert_preserves_non_empty_read_bits() {
    let event = EventEnvelope::new(
        crate::state::test_channel_id(4),
        Seq(11),
        EventKind::PostUpsert,
        crate::sync_session::PostFields {
            id: "post-online".to_string(),
            temporary_id: "tmp-online".to_string(),
            read_bits: String::new(),
            ..Default::default()
        },
    );
    let StorageOp::BatchUpsert(spec) = event_to_storage_op(&event) else {
        panic!("online post must use BatchUpsert");
    };
    assert!(spec.exclude_from_update.contains(&"read_bits"));
}

/// gap timer 预览的 type=1 属于 sync 权威路径，空 read_bits 不得覆盖已读位。
#[test]
fn gap_peek_post_upsert_preserves_non_empty_read_bits() {
    let mut channel = Channel::new(crate::state::test_channel_id(5), 0);
    let mut effects = EffectSink::new();
    channel
        .ingest(gapped_ev(5, 2), &mut effects, 1_000)
        .expect("buffer gapped post");
    let operations = channel.peek_up_to(Seq(2));
    let StorageOp::BatchUpsert(spec) = &operations[0] else {
        panic!("gapped post must use BatchUpsert");
    };
    assert!(spec.exclude_from_update.contains(&"read_bits"));
}

fn is_too_long_emit(e: &Effect) -> bool {
    matches!(e, Effect::Emit { event }
            if String::from_utf8_lossy(event.0.as_ref()).contains("im:sync:too_long"))
}

/// 回归锚: E6 —— gate buffer 客户端上限。
///
/// 制造永久缺口（全部 seq >= 2，缺 seq=1），持续灌入远超 MAX_GATE_BUFFER 的
/// 乱序事件。断言：① buffer 始终有界（永不超过上限）；② 越限时主动
/// emit im:sync:too_long；③ cursor 不被回退（安全网靠 sync 从 cursor+1 重拉）。
#[test]
fn test_gate_buffer_capped_proactive_too_long_e6() {
    let mut ch = Channel::new(crate::state::test_channel_id(7), 0); // cursor=0 → expected=1，制造永久缺口
    let mut fx = EffectSink::new();
    let mut saw_too_long = false;
    let mut saw_message_delete = false;

    // 灌到越过上限 + 余量，确保触发至少一次主动恢复
    for seq in 2..(MAX_GATE_BUFFER as u64 + 2 + 128) {
        fx.clear();
        ch.ingest(gapped_ev(7, seq), &mut fx, 1_000)
            .expect("ingest gapped event");
        assert!(
            ch.buffer.len() <= MAX_GATE_BUFFER,
            "buffer 必须有界（≤ 上限），实际 {}",
            ch.buffer.len()
        );
        if fx.as_slice().iter().any(is_too_long_emit) {
            saw_too_long = true;
        }
        saw_message_delete |= fx.as_slice().iter().any(|effect| match effect {
            Effect::Persist { ops, .. } | Effect::PersistFire { ops } => ops.iter().any(|op| {
                let op = format!("{op:?}");
                op.contains("DeleteWhere") || (op.contains("BatchDelete") && op.contains("message"))
            }),
            _ => false,
        });
    }

    assert!(
        saw_too_long,
        "越过 buffer 上限必须主动 emit im:sync:too_long（不再静等服务端）"
    );
    assert!(
        !saw_message_delete,
        "越过 buffer 上限不得生成 DeleteWhere；reload 失败时必须保留旧 message 历史"
    );
    assert_eq!(
        ch.cursor.value(),
        Seq(0),
        "E6 安全网不回退 cursor（缺口由后续 sync 从 cursor+1 重拉）"
    );
}

/// 上限以内正常乱序堆积不误触发 too_long（不误伤承压合法 5000 缺口场景）。
#[test]
fn test_gate_buffer_under_cap_no_false_too_long_e6() {
    let mut ch = Channel::new(crate::state::test_channel_id(3), 0);
    let mut fx = EffectSink::new();

    for seq in 2..=5001 {
        // 承压同款 5000 缺口
        ch.ingest(gapped_ev(3, seq), &mut fx, 1_000)
            .expect("ingest");
    }
    assert_eq!(ch.buffer.len(), 5000, "5000 缺口应全部正常堆积");
    assert!(
        !fx.as_slice().iter().any(is_too_long_emit),
        "上限以内不得误触发 too_long"
    );
}

/// 携 unread_bump（delta=1 他人可见）的 PostUpsert 事件构造器。
fn bump_ev(ch: u64, seq: u64) -> EventEnvelope {
    let upd = crate::channel_write::PostChannelUpdate {
        channel_id: crate::state::test_channel_id(ch),
        unread_delta: 1,
        unread_post_id: Some("p".to_string()),
        last_post: "x".to_string(),
        has_schedule_post: false,
        msg_create_at: 1_000,
        visible: true,
        sender_user_id: "other".to_string(),
        post_id: "p".to_string(),
        last_message: "x".to_string(),
        mentions: Vec::new(),
        urgent_user_ids: Vec::new(),
        mention_hit: false,
        urgent_hit: false,
    };
    EventEnvelope::new(
        crate::state::test_channel_id(ch),
        Seq(seq),
        EventKind::PostUpsert,
        crate::sync_session::PostFields::default(),
    )
    .with_unread_bump(Some(upd))
}

fn is_unread_bump(e: &Effect) -> bool {
    matches!(e, Effect::PersistFire { ops }
            if ops.iter().any(|op| matches!(op, StorageOp::GuardedBump(_))))
}

/// Canonical stream 只负责消息事实与 cursor；未读只由 member_projection 绝对覆盖。
#[test]
fn out_of_order_post_flush_does_not_infer_unread() {
    let mut ch = Channel::new(crate::state::test_channel_id(9), 0);
    let mut fx = EffectSink::new();

    // seq=2 乱序 → buffer，携 unread_bump 但未 apply → 不得提前 bump 未读。
    ch.ingest(bump_ev(9, 2), &mut fx, 1_000)
        .expect("buffer gapped");
    assert!(
        !fx.as_slice().iter().any(is_unread_bump),
        "乱序未 apply 不应提前 bump 未读"
    );

    // seq=1 填平后两条消息仍会 apply/flush，但不得产生本地未读 delta。
    fx.clear();
    ch.ingest(bump_ev(9, 1), &mut fx, 1_000)
        .expect("apply + flush");
    assert!(
        !fx.as_slice().iter().any(is_unread_bump),
        "stream apply/flush 不得绕过 member_projection 推断未读"
    );
}

/// 回归锚: ⑤ —— `unread_bump=None`（echo / sync 路径）的事件 flush 时**不**补未读。
/// 防回归：gate 只发出携带的决策，不擅自为无决策事件 +1（避免双计或 echo 误计）。
#[test]
fn test_none_unread_bump_does_not_increment_5() {
    let mut ch = Channel::new(crate::state::test_channel_id(11), 0);
    let mut fx = EffectSink::new();
    // 默认 unread_bump=None（EventEnvelope::new）。
    ch.ingest(gapped_ev(11, 2), &mut fx, 1_000).expect("buffer");
    fx.clear();
    ch.ingest(gapped_ev(11, 1), &mut fx, 1_000)
        .expect("apply + flush");
    assert!(
        !fx.as_slice().iter().any(is_unread_bump),
        "unread_bump=None 的事件 apply/flush 都不得 bump 未读"
    );
}

#[test]
fn duplicate_protocol_frame_is_rejected_while_stream_seq_is_inflight() {
    let mut ch = Channel::new(crate::state::test_channel_id(12), 0);
    let mut fx = EffectSink::new();

    let first = ch
        .admit_message_v3_post(gapped_ev(12, 1), &mut fx)
        .expect("admit canonical");
    let duplicate = ch
        .admit_message_v3_post(gapped_ev(12, 1), &mut fx)
        .expect("admit legacy duplicate");

    assert!(first.is_some());
    assert!(duplicate.is_none());

    let failed = first.expect("first event");
    ch.restore_message_v3_post(failed, &mut fx);
    assert!(
        ch.admit_message_v3_post(gapped_ev(12, 1), &mut fx)
            .expect("retry after failed persist")
            .is_some(),
        "失败回执必须释放在途水位，允许同序号补偿重试"
    );
}

/// 回归锚: hydration TooLong——覆盖窗口只能删除已覆盖前缀，保留恢复期间到达的新事件。
#[test]
fn too_long_atomic_commit_preserves_events_above_recovery_head() {
    let mut channel = Channel::new(crate::state::test_channel_id(1), 0);
    channel.buffer.insert(Seq(99), gapped_ev(1, 99));
    channel.buffer.insert(Seq(101), gapped_ev(1, 101));
    assert!(channel.commit_too_long_after_atomic(Seq(100)));
    assert_eq!(channel.cursor.value(), Seq(100));
    assert!(!channel.buffer.contains_key(&Seq(99)));
    assert!(channel.buffer.contains_key(&Seq(101)));
}

/// 回归锚: hydration TooLong——已提交的在线更高水位不能被迟到恢复窗口倒退或误报失败。
#[test]
fn too_long_atomic_commit_keeps_newer_live_cursor() {
    let mut channel = Channel::new(crate::state::test_channel_id(1), 101);
    channel.buffer.insert(Seq(103), gapped_ev(1, 103));
    assert!(channel.commit_too_long_after_atomic(Seq(100)));
    assert_eq!(channel.cursor.value(), Seq(101));
    assert!(channel.buffer.contains_key(&Seq(103)));
}
