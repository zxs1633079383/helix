//! Regression: receipt events occupy the canonical stream even while storage is pending.
mod common;
use common::*;
use helix_core::{
    tick::{InboundBytes, PortOutcome},
    Tick,
};

/// Stream fixtures intentionally include channel-level type6 without a message id.
fn stream(seq: u64, redacted: bool) -> Tick {
    Tick::Inbound(InboundBytes(
        serde_json::to_vec(&serde_json::json!({
            "action": "channel_stream_event", "data": {
                "channel_id": cid(1), "stream_seq": seq, "event_id": format!("event-{seq}"),
                "event_type": if seq == 1 {6} else {1}, "redacted": redacted
            }
        }))
        .unwrap()
        .into(),
    ))
}

/// Extract the single serialized durable write without fabricating a correlation.
fn atomic(effects: &[EffectKind]) -> u64 {
    let ids: Vec<_> = effects
        .iter()
        .filter_map(|e| match e {
            EffectKind::PersistAtomic(c) => Some(*c),
            _ => None,
        })
        .collect();
    assert_eq!(ids.len(), 1, "{effects:?}");
    ids[0]
}

/// An ordered burst waits locally and drains one acknowledged event at a time.
#[test]
fn contiguous_burst_waits_without_gap_and_reserves_every_next_slot() {
    for redacted in [false, true] {
        let mut h = Harness::new_with_identity(&[1], "author", "tenant", "", "", "");
        let mut corr = atomic(&h.step(stream(1, redacted), 1000));
        for seq in 2..=32 {
            let effects = h.step(stream(seq, redacted), 1000 + seq);
            assert!(
                effects.is_empty(),
                "ordered event {seq} is not a gap: {effects:?}"
            );
        }
        for seq in 1..=32 {
            assert_eq!(h.module.cursor_for(channel_id(1)), Some(seq - 1));
            let effects = h.step(port_reply_ok(corr), 2000 + seq);
            assert_eq!(h.module.cursor_for(channel_id(1)), Some(seq));
            if seq < 32 {
                corr = atomic(&effects);
                assert!(
                    h.step(stream(seq + 1, redacted), 3000 + seq).is_empty(),
                    "pending duplicate must not write again"
                );
            } else {
                assert!(!effects.iter().any(|e| matches!(
                    e,
                    EffectKind::PersistAtomic(_)
                        | EffectKind::Http(_)
                        | EffectKind::ScheduleTimer(_)
                )));
            }
        }
    }
}

/// A genuinely absent sequence still requests server recovery immediately.
#[test]
fn missing_sequence_still_recovers() {
    let mut h = Harness::new_with_identity(&[1], "author", "tenant", "", "", "");
    h.step(stream(1, true), 1000);
    let effects = h.step(stream(3, true), 1001);
    assert!(
        effects.iter().any(|e| matches!(e, EffectKind::Http(_))),
        "{effects:?}"
    );
    assert_eq!(h.module.cursor_for(channel_id(1)), Some(0));
}

/// Failure must not commit the cursor and must retain a recovery timer.
#[test]
fn persistence_failure_retains_cursor_and_recovers() {
    let mut h = Harness::new_with_identity(&[1], "author", "tenant", "", "", "");
    let corr = atomic(&h.step(stream(1, true), 1000));
    h.step(stream(2, true), 1001);
    let effects = h.step(
        Tick::PortReply {
            corr: helix_core::Correlation::from_raw(corr),
            outcome: PortOutcome::Err(helix_core::tick::PortError::Storage(1)),
        },
        1002,
    );
    assert_eq!(h.module.cursor_for(channel_id(1)), Some(0));
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, EffectKind::ScheduleTimer(_))),
        "{effects:?}"
    );
    let timer = effects
        .iter()
        .find_map(|e| match e {
            EffectKind::ScheduleTimer(t) => Some(*t),
            _ => None,
        })
        .unwrap();
    let recovery = h.step(timer_tick(timer), 2003);
    assert!(recovery.iter().any(|e| matches!(e, EffectKind::Http(_))));
    assert_eq!(h.module.cursor_for(channel_id(1)), Some(0));
}

/// Filling an observed hole cancels its timer while the contiguous writes are pending.
#[test]
fn filled_hole_cancels_timer_and_drains_in_order() {
    let mut h = Harness::new_with_identity(&[1], "author", "tenant", "", "", "");
    let first = atomic(&h.step(stream(1, true), 1000));
    let gap = h.step(stream(3, true), 1001);
    let timer = gap
        .iter()
        .find_map(|e| match e {
            EffectKind::ScheduleTimer(t) => Some(*t),
            _ => None,
        })
        .unwrap();
    let filled = h.step(stream(2, true), 1002);
    assert!(
        filled.contains(&EffectKind::CancelTimer(timer)),
        "{filled:?}"
    );
    assert!(h.step(timer_tick(timer), 2002).is_empty());
    let second = atomic(&h.step(port_reply_ok(first), 2003));
    let third = atomic(&h.step(port_reply_ok(second), 2004));
    h.step(port_reply_ok(third), 2005);
    assert_eq!(h.module.cursor_for(channel_id(1)), Some(3));
}

/// Legacy author receipt can arrive first; only canonical acknowledgement advances cursor.
#[test]
fn author_receipt_then_canonical_then_message_is_contiguous() {
    let mut h = Harness::new_with_identity(&[1], "author", "tenant", "", "", "");
    let receipt = Tick::Inbound(InboundBytes(
        serde_json::to_vec(&serde_json::json!({
            "action":"post_read", "data": {
                "channelId":cid(1), "postId":sid(1), "eventSeq":1,
                "authorUserId":"author", "readerUserId":"reader", "snapshotId":"snapshot",
                "memberIds":["reader"], "readMap":"1", "updateAt":10
            }
        }))
        .unwrap()
        .into(),
    ));
    let effects = h.step(receipt, 1000);
    let read_corr = effects
        .iter()
        .find_map(|e| match e {
            EffectKind::Persist(c) => Some(*c),
            _ => None,
        })
        .unwrap();
    h.step(port_reply_ok(read_corr), 1001);
    assert_eq!(h.module.cursor_for(channel_id(1)), Some(0));
    let first = atomic(&h.step(stream(1, false), 1002));
    assert!(h.step(stream(2, false), 1003).is_empty());
    let second = atomic(&h.step(port_reply_ok(first), 1004));
    h.step(port_reply_ok(second), 1005);
    assert_eq!(h.module.cursor_for(channel_id(1)), Some(2));
}

/// PRE 的接龙通用事件只有 chain payload：必须先查完整 post，不能落 evt 空白行。
#[test]
fn chain_stream_requires_authoritative_post_before_persist() {
    let mut h = Harness::new_with_identity(&[1], "viewer", "tenant", "", "", "");
    let effects = h.step(Tick::Inbound(InboundBytes(serde_json::to_vec(&serde_json::json!({
        "action":"channel_stream_event", "data": {
            "channel_id":cid(1), "stream_seq":1, "event_id":"chain-publish-1",
            "event_type":1, "msg_id":sid(1), "actor_id":"author", "redacted":false,
            "payload":{"action":"im:post_chain:publish", "chain_summary":{
                "chainId":"chain-one", "channelId":cid(1), "declarationPostId":sid(1), "title":"吃饭啦"
            }}
        }
    })).unwrap().into())), 1000);
    assert!(
        effects.iter().any(|e| matches!(e, EffectKind::Http(_))),
        "{effects:?}"
    );
    assert!(
        !effects
            .iter()
            .any(|e| matches!(e, EffectKind::PersistAtomic(_))),
        "{effects:?}"
    );
    assert_eq!(h.module.cursor_for(channel_id(1)), Some(0));
    assert_eq!(
        h.last_http_body().unwrap(),
        serde_json::json!({"postIds":[sid(1)]})
    );
}

/// 使用 PRE 同形的接龙发布/追加帧，完整消息仅由 posts/get 返回。
fn chain_stream_payload(action: &str) -> Tick {
    Tick::Inbound(InboundBytes(serde_json::to_vec(&serde_json::json!({
        "action":"channel_stream_event", "data": {
            "channel_id":cid(1), "stream_seq":1, "event_id":"chain-event-1",
            "event_type":1, "msg_id":sid(1), "actor_id":"author", "redacted":false,
            "payload":{"action":action, "chain_summary":{"chainId":"chain-one", "channelId":cid(1)}}
        }
    })).unwrap().into()))
}

/// 权威正文回包包含可见集和服务端消息身份，不从 chainSummary 猜消息。
fn chain_post_reply(corr: u64, channel: &str, post: &str) -> Tick {
    let body = serde_json::to_vec(&serde_json::json!({"status":"SUCCESS", "data":[{
        "id":post, "channelId":channel, "userId":"author", "teamId":"tenant", "type":"TEXT_CHAIN",
        "message":"文字接龙：吃饭啦", "createAt":100, "viewers":["all"],
        "props":{"chain":{"chainId":"chain-one", "mode":"TEXT", "messageRole":"DECLARATION", "title":"吃饭啦"}}
    }]})).unwrap();
    Tick::PortReply {
        corr: helix_core::Correlation::from_raw(corr),
        outcome: PortOutcome::Ok(helix_core::tick::ReplyBytes(enveloped(body.into()))),
    }
}

/// 发布/追加均先读权威 body，再原子落库；重复与迟到回包不能重复写入或 Emit。
#[test]
fn chain_publish_and_append_hydrate_then_commit_once() {
    for action in ["im:post_chain:publish", "im:post_chain:upsert"] {
        let mut h = Harness::new_with_identity(&[1], "viewer", "tenant", "", "", "");
        h.seed_channel_scope(&cid(1), "tenant", "viewer");
        let effects = h.step(chain_stream_payload(action), 1000);
        let corr = effects
            .iter()
            .find_map(|e| {
                if let EffectKind::Http(c) = e {
                    Some(*c)
                } else {
                    None
                }
            })
            .unwrap();
        assert!(h.step(chain_stream_payload(action), 1001).is_empty());
        let writes = h.step(chain_post_reply(corr, &cid(1), &sid(1)), 1002);
        let write_corr = atomic(&writes);
        let serialized = format!("{:?}", h.sink.as_slice());
        assert!(
            serialized.contains("TEXT_CHAIN")
                && serialized.contains("吃饭啦")
                && serialized.contains(&sid(1)),
            "{serialized}"
        );
        assert!(!serialized.contains("evt:"));
        assert!(!writes.contains(&EffectKind::Emit));
        let committed = h.step(port_reply_ok(write_corr), 1003);
        assert!(committed.contains(&EffectKind::Emit));
        assert_eq!(h.module.cursor_for(channel_id(1)), Some(1));
        assert!(h
            .step(chain_post_reply(corr, &cid(1), &sid(1)), 1004)
            .is_empty());
        assert!(h.step(chain_stream_payload(action), 1005).is_empty());
        assert!(h.step(port_reply_ok(write_corr), 1006).is_empty());
    }
}

/// 错频道、错 postId、业务拒绝和网络失败不能落库或释放消息，保留补偿入口。
#[test]
fn chain_hydration_rejects_wrong_scope_identity_and_transport() {
    for failure in 0..5 {
        let mut h = Harness::new_with_identity(&[1], "viewer", "tenant", "", "", "");
        let effects = h.step(chain_stream_payload("im:post_chain:publish"), 1000);
        let corr = effects
            .iter()
            .find_map(|e| {
                if let EffectKind::Http(c) = e {
                    Some(*c)
                } else {
                    None
                }
            })
            .unwrap();
        let reply = match failure {
            0 => chain_post_reply(corr, &cid(2), &sid(1)),
            1 => chain_post_reply(corr, &cid(1), &sid(2)),
            2 => Tick::PortReply {
                corr: helix_core::Correlation::from_raw(corr),
                outcome: PortOutcome::Err(helix_core::tick::PortError::Network),
            },
            3 => Tick::PortReply {
                corr: helix_core::Correlation::from_raw(corr),
                outcome: PortOutcome::Err(helix_core::tick::PortError::Timeout),
            },
            _ => Tick::PortReply {
                corr: helix_core::Correlation::from_raw(corr),
                outcome: PortOutcome::Ok(helix_core::tick::ReplyBytes(enveloped(
                    bytes::Bytes::from_static(b"{\"status\":\"FAIL\",\"data\":[]}"),
                ))),
            },
        };
        let rejected = h.step(reply, 1001);
        assert!(
            !rejected
                .iter()
                .any(|e| matches!(e, EffectKind::PersistAtomic(_) | EffectKind::Emit)),
            "{rejected:?}"
        );
        assert_eq!(h.module.cursor_for(channel_id(1)), Some(0));
        assert!(
            rejected
                .iter()
                .any(|e| matches!(e, EffectKind::ScheduleTimer(_))),
            "{rejected:?}"
        );
    }
}

/// 关闭会话必须撤销未完成的补全读取，迟到 HTTP 不得再写库或发布消息。
#[test]
fn chain_hydration_late_reply_after_stop_is_ignored() {
    use helix_core::Module;
    let mut h = Harness::new_with_identity(&[1], "viewer", "tenant", "", "", "");
    let effects = h.step(chain_stream_payload("im:post_chain:publish"), 1000);
    let corr = effects
        .iter()
        .find_map(|e| {
            if let EffectKind::Http(c) = e {
                Some(*c)
            } else {
                None
            }
        })
        .unwrap();
    h.sink.clear();
    h.module.on_stop(&mut h.sink).unwrap();
    let late = h.step(chain_post_reply(corr, &cid(1), &sid(1)), 1001);
    assert!(late.is_empty(), "{late:?}");
}

/// 完整 body 读成功仍不等于提交成功：Storage Err 与迟到 ack 不能发布消息。
#[test]
fn chain_hydration_persist_failure_keeps_message_uncommitted() {
    let mut h = Harness::new_with_identity(&[1], "viewer", "tenant", "", "", "");
    let first = h.step(chain_stream_payload("im:post_chain:publish"), 1000);
    let corr = first
        .iter()
        .find_map(|e| {
            if let EffectKind::Http(c) = e {
                Some(*c)
            } else {
                None
            }
        })
        .unwrap();
    let writes = h.step(chain_post_reply(corr, &cid(1), &sid(1)), 1001);
    let corr = atomic(&writes);
    let failed = h.step(
        Tick::PortReply {
            corr: helix_core::Correlation::from_raw(corr),
            outcome: PortOutcome::Err(helix_core::tick::PortError::Storage(1)),
        },
        1002,
    );
    assert!(!failed.contains(&EffectKind::Emit));
    assert!(failed
        .iter()
        .any(|e| matches!(e, EffectKind::ScheduleTimer(_))));
    assert_eq!(h.module.cursor_for(channel_id(1)), Some(0));
    assert!(h.step(port_reply_ok(corr), 1003).is_empty());
}
