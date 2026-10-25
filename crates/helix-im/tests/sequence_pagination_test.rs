//! 回归锚：固定 post.seq 的本地分页与不可见占位；只验证 sans-IO 生命周期。
mod common;

use bytes::Bytes;
use common::{cid, sid, Harness};
use helix_core::tick::{PortOutcome, ReplyBytes};
use helix_core::{Effect, Module, Tick};
use serde_json::json;
use serde_json::Value;

/// Repeatable public command fixture; no private reducer/service calls.
fn start(h: &mut Harness) -> (helix_core::Correlation, helix_core::TimerId) {
    let command=helix_im::client_api::build_command("im_load_older_context",json!({"channel_id":cid(1),"anchor_post_id":sid(20),"pageSize":20,"req_id":"seq-page","sequencePagination":true}).to_string().as_bytes()).unwrap();
    h.step(Tick::Command(command), 1);
    let corr = h
        .sink
        .as_slice()
        .iter()
        .find_map(|e| {
            if let Effect::Http { corr, .. } = e {
                Some(*corr)
            } else {
                None
            }
        })
        .unwrap();
    let timer = h
        .sink
        .as_slice()
        .iter()
        .find_map(|e| {
            if let Effect::ScheduleTimer { id, .. } = e {
                Some(*id)
            } else {
                None
            }
        })
        .unwrap();
    (corr, timer)
}

/// Decode emitted production events only.
fn emitted(h: &Harness) -> Vec<Value> {
    h.sink
        .as_slice()
        .iter()
        .filter_map(|e| {
            if let Effect::Emit { event } = e {
                Some(serde_json::from_slice(&event.0).unwrap())
            } else {
                None
            }
        })
        .collect()
}

/// Transport failure is terminal and late/duplicate replies cannot revive it.
#[test]
fn network_error_timeout_and_stop_release_pending_without_fallback() {
    for action in 0..3 {
        let mut h = Harness::new(&[1]);
        let (corr, timer) = start(&mut h);
        match action {
            0 => {
                h.step(
                    Tick::PortReply {
                        corr,
                        outcome: PortOutcome::Err(helix_core::tick::PortError::Network),
                    },
                    2,
                );
            }
            1 => {
                h.step(Tick::Timer(timer), 10_001);
            }
            _ => {
                h.sink.clear();
                h.module.on_stop(&mut h.sink).unwrap();
            }
        }
        assert!(emitted(&h).iter().any(|v| v["data"]["error"].is_string()));
        h.step(
            Tick::PortReply {
                corr,
                outcome: PortOutcome::Ok(ReplyBytes(Bytes::from_static(b"{}"))),
            },
            20_000,
        );
        assert!(emitted(&h).is_empty());
        start(&mut h); // same reqId can be reused after every terminal path
    }
}

/// Only the absent versioned route or explicit not-ready capability selects legacy paging.
#[test]
fn old_service_404_falls_back_but_401_and_503_do_not() {
    for status in [404, 401, 503] {
        let mut h = Harness::new(&[1]);
        let (corr, _) = start(&mut h);
        h.step(
            Tick::PortReply {
                corr,
                outcome: PortOutcome::Ok(ReplyBytes(Bytes::from(
                    json!({"status":status,"headers":[],"body":"e30="}).to_string(),
                ))),
            },
            2,
        );
        let legacy = h.sink.as_slice().iter().any(
            |e| matches!(e,Effect::Http{req,..} if req.url.ends_with("/posts/getPostsAfterIndex")),
        );
        assert_eq!(legacy, status == 404);
        if status != 404 {
            assert!(emitted(&h)[0]["data"]["error"].is_string());
        }
    }
}

/// V2 cursors reject lossy numbers, overflow and cross-channel reuse before I/O.
#[test]
fn untrusted_cursor_never_becomes_a_partial_or_cross_channel_query() {
    for value in [
        json!(9007199254740993_u64),
        json!("9223372036854775808"),
        json!("01"),
        json!("-1"),
    ] {
        let mut h = Harness::new(&[1]);
        let command=helix_im::client_api::build_command("im_load_older_context",json!({"channel_id":cid(1),"pageSize":20,"req_id":"invalid","sequencePagination":true,"cursor":{"version":2,"channelId":cid(1),"direction":"older","nextSeq":value,"headSeq":"100","membershipEpoch":"1","revision":"1"}}).to_string().as_bytes()).unwrap();
        assert!(h
            .module
            .handle(&Tick::Command(command), 1, &mut h.sink)
            .is_err());
        assert!(!h.sink.as_slice().iter().any(|e| matches!(
            e,
            Effect::Http { .. } | Effect::Persist { .. } | Effect::PersistAtomic { .. }
        )));
    }
}

/// 新消费者显式选择新协议，旧命令不改变默认请求。
#[test]
fn sequence_pagination_probes_versioned_authority() {
    let mut h = Harness::new(&[1]);
    let command = helix_im::client_api::build_command(
        "im_load_older_context",
        json!({
            "channel_id": cid(1), "anchor_post_id": sid(20), "pageSize":20,
            "req_id":"seq-page", "sequencePagination":true
        })
        .to_string()
        .as_bytes(),
    )
    .unwrap();
    assert!(h.module.accepts(&Tick::Command(command.clone())));
    h.step(Tick::Command(command), 1);
    assert!(h.sink.as_slice().iter().any(|effect| matches!(effect,
        Effect::Http { req, .. } if req.url.ends_with("/posts/sequenceRange"))));
}

/// Go's legacy router uses HTTP 200 for missing routes; access failures remain terminal.
#[test]
fn exact_old_router_and_disabled_capability_fall_back_without_hiding_access_errors() {
    for (body, fallback) in [
        (json!({"status":"failed","message":"not found"}), true),
        (
            json!({"status":"SUCCESS","data":{"supported":true,"ready":false,"slots":[]}}),
            true,
        ),
        (
            json!({"status":"failed","message":"SEQUENCE_ACCESS_DENIED","data":{"code":"SEQUENCE_ACCESS_DENIED"}}),
            false,
        ),
        (
            json!({"status":"failed","message":"not found","data":{"code":"SEQUENCE_ACCESS_DENIED"}}),
            false,
        ),
    ] {
        let mut h = Harness::new(&[1]);
        let (corr, _) = start(&mut h);
        h.step(
            Tick::PortReply {
                corr,
                outcome: PortOutcome::Ok(ReplyBytes(common::enveloped(Bytes::from(
                    body.to_string(),
                )))),
            },
            2,
        );
        assert_eq!(h.sink.as_slice().iter().any(|e|matches!(e,Effect::Http{req,..} if req.url.ends_with("/posts/getPostsAfterIndex"))),fallback);
        if !fallback {
            assert!(emitted(&h)[0]["data"]["error"].is_string());
        }
    }
}

/// Revision changes can make already-scanned invisible slots visible, so old cursors must restart.
#[test]
fn changed_revision_expires_continuation_before_local_read() {
    let mut h = Harness::new(&[1]);
    let command=helix_im::client_api::build_command("im_load_older_context",json!({"channel_id":cid(1),"req_id":"cursor","sequencePagination":true,"cursor":{"version":2,"channelId":cid(1),"direction":"older","nextSeq":"80","headSeq":"100","membershipEpoch":"1","revision":"1"}}).to_string().as_bytes()).unwrap();
    h.step(Tick::Command(command), 1);
    let corr = h
        .sink
        .as_slice()
        .iter()
        .find_map(|e| {
            if let Effect::Http { corr, .. } = e {
                Some(*corr)
            } else {
                None
            }
        })
        .unwrap();
    h.step(Tick::PortReply{corr,outcome:PortOutcome::Ok(ReplyBytes(common::enveloped(Bytes::from(json!({"status":"SUCCESS","data":{"supported":true,"ready":true,"headSeq":"100","snapshotHeadSeq":"100","firstSeq":"1","membershipEpoch":"1","revision":"2"}}).to_string()))))},2);
    assert!(emitted(&h).iter().any(|event| event["data"]["error"]
        .as_str()
        .is_some_and(|s| s.contains("CURSOR_EXPIRED"))));
    assert!(!h.sink.as_slice().iter().any(|effect| matches!(
        effect,
        Effect::Persist { .. } | Effect::PersistAtomic { .. }
    )));
}

/// A newer authority observed by another query fences late metadata even for disjoint ranges.
#[test]
fn late_metadata_cannot_lower_the_channel_authority_revision() {
    let mut h = Harness::new(&[1]);
    let (old, _) = start(&mut h);
    let command=helix_im::client_api::build_command("im_load_older_context",json!({"channel_id":cid(1),"anchor_post_id":sid(90),"req_id":"new-meta","sequencePagination":true}).to_string().as_bytes()).unwrap();
    h.step(Tick::Command(command), 2);
    let newer = h
        .sink
        .as_slice()
        .iter()
        .find_map(|e| {
            if let Effect::Http { corr, .. } = e {
                Some(*corr)
            } else {
                None
            }
        })
        .unwrap();
    for (corr, revision, anchor) in [(newer, "2", "90"), (old, "1", "20")] {
        h.step(Tick::PortReply{corr,outcome:PortOutcome::Ok(ReplyBytes(common::enveloped(Bytes::from(json!({"status":"SUCCESS","data":{"supported":true,"ready":true,"headSeq":"100","snapshotHeadSeq":"100","firstSeq":"1","membershipEpoch":"1","revision":revision,"anchorSeq":anchor}}).to_string()))))},3);
    }
    assert!(emitted(&h).iter().any(|event| event["data"]["error"]
        .as_str()
        .is_some_and(|s| s.contains("SNAPSHOT_CHANGED"))));
    assert!(!h.sink.as_slice().iter().any(|effect| matches!(
        effect,
        Effect::Persist { .. } | Effect::PersistAtomic { .. }
    )));
    h.sink.clear();
    h.module.on_stop(&mut h.sink).unwrap();
}

/// Detached selection is a locate-only option and cannot leak into the strict old protocol.
#[test]
fn detached_locator_flag_is_scoped_and_legacy_fallback_removes_it() {
    for (name, attach) in [
        ("im_query_messages_by_channel", json!(false)),
        ("im_locate_context", json!("false")),
    ] {
        let mut h = Harness::new(&[1]);
        let command = helix_im::client_api::build_command(name, json!({"channel_id":cid(1),"target_post_id":sid(20),"req_id":"detached","sequencePagination":true,"attach":attach}).to_string().as_bytes()).unwrap();
        assert!(h
            .module
            .handle(&Tick::Command(command), 1, &mut h.sink)
            .is_err());
        assert!(h.sink.as_slice().is_empty());
    }
    let mut h = Harness::new(&[1]);
    let command = helix_im::client_api::build_command("im_locate_context", json!({"channel_id":cid(1),"target_post_id":sid(20),"req_id":"detached","sequencePagination":true,"attach":false}).to_string().as_bytes()).unwrap();
    h.step(Tick::Command(command), 1);
    let corr = h
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Http { corr, .. } => Some(*corr),
            _ => None,
        })
        .unwrap();
    h.step(
        Tick::PortReply {
            corr,
            outcome: PortOutcome::Ok(ReplyBytes(common::enveloped(Bytes::from(
                json!({"status":"SUCCESS","data":{"supported":true,"ready":false,"slots":[]}})
                    .to_string(),
            )))),
        },
        2,
    );
    assert!(emitted(&h)
        .iter()
        .all(|event| event["data"].get("error").is_none()));
    assert!(
        h.sink
            .as_slice()
            .iter()
            .any(|effect| matches!(effect, Effect::Persist { .. } | Effect::Http { .. })),
        "legacy locate must continue through its public read path"
    );
    h.sink.clear();
    h.module.on_stop(&mut h.sink).unwrap();
}
