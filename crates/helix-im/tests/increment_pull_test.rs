//! 分批恢复公开接缝：下一页受当前页 PersistOk 背压，不能提前广播完成。
mod common;
use common::*;
use helix_core::tick::InboundBytes;
use helix_core::{Effect, Tick};
use serde_json::json;

/// 通过具有分页能力的真实形态 hello 启动恢复。
fn start_pull(h: &mut Harness) -> u64 {
    h.start(0);
    h.step(Tick::Inbound(InboundBytes(serde_json::to_vec(&json!({"action":"hello","data":{"connectionId":"pull-conn","incrementPageVersion":1}})).unwrap().into())), 1);
    let scan = message_timestamp_scan_corr(&h.sink).expect("watermark scan");
    h.step(port_reply_with_body(scan, b"[]".as_slice().into()), 2);
    h.sink
        .as_slice()
        .iter()
        .find_map(|e| match e {
            Effect::Http { corr, req } if req.url.ends_with("/channels/load/increment/page") => {
                Some(corr.raw())
            }
            _ => None,
        })
        .expect("capability must select pull recovery")
}

/// 冻结65个ID时只处理首64份投影，持久化成功才取最后一页。
#[test]
fn increment_pull_waits_for_persist_before_next_page() {
    let mut h = Harness::new(&[]);
    let corr = start_pull(&mut h);
    let ids: Vec<_> = (1..=65).map(cid).collect();
    let channels: Vec<_> = ids[..64]
        .iter()
        .map(|id| json!({"id":id,"lastEventSeq":0,"needSync":false}))
        .collect();
    let body = enveloped(
        serde_json::to_vec(
            &json!({"status":"SUCCESS","data":{"manifest":ids,"channels":channels}}),
        )
        .unwrap()
        .into(),
    );
    h.step(port_reply_with_body(corr, body), 3);
    assert!(
        !h.sink
            .as_slice()
            .iter()
            .any(|e| matches!(e, Effect::Http { .. } | Effect::Emit { .. })),
        "page must wait for durable storage"
    );
    let persist = h
        .sink
        .as_slice()
        .iter()
        .find_map(|e| match e {
            Effect::PersistAtomic { corr, .. } => Some(corr.raw()),
            _ => None,
        })
        .expect("page atomic barrier");
    h.step(port_reply_ok(persist), 4);
    let next = h
        .sink
        .as_slice()
        .iter()
        .find_map(|e| match e {
            Effect::Http { req, .. } => Some(req),
            _ => None,
        })
        .expect("next page after ack");
    let body: serde_json::Value = serde_json::from_slice(next.body.as_ref().unwrap()).unwrap();
    assert_eq!(body["channelIds"], json!([cid(65)]));
}

/// 持久化失败必须停止续页，重复失败回执不能释放第二份请求。
#[test]
fn increment_pull_persist_failure_does_not_continue() {
    let mut h = Harness::new(&[]);
    let corr = start_pull(&mut h);
    let ids: Vec<_> = (1..=65).map(cid).collect();
    let channels: Vec<_> = ids[..64]
        .iter()
        .map(|id| json!({"id":id,"needSync":false}))
        .collect();
    let body = enveloped(
        serde_json::to_vec(
            &json!({"status":"SUCCESS","data":{"manifest":ids,"channels":channels}}),
        )
        .unwrap()
        .into(),
    );
    h.step(port_reply_with_body(corr, body), 3);
    let persist = h
        .sink
        .as_slice()
        .iter()
        .find_map(|e| match e {
            Effect::PersistAtomic { corr, .. } => Some(corr.raw()),
            _ => None,
        })
        .expect("page atomic barrier");
    assert!(
        h.module.cursor_for(channel_id(1)).is_none(),
        "uncommitted page mutated channel state"
    );
    h.step(port_reply_err(persist), 4);
    assert!(
        h.module.cursor_for(channel_id(1)).is_none(),
        "failed page mutated channel state"
    );
    assert!(h.sink.as_slice().is_empty());
    h.step(port_reply_ok(persist), 5);
    assert!(h.sink.as_slice().is_empty());
}

/// 2436频道超过旧2048队列时仍严格64一页，39次持久化完成前不放行下一页。
#[test]
fn increment_pull_2436_channels_has_one_page_inflight() {
    let mut h = Harness::new(&[]);
    let mut corr = start_pull(&mut h);
    let ids: Vec<_> = (1..=2436).map(cid).collect();
    for (page, chunk) in ids.chunks(64).enumerate() {
        let channels: Vec<_> = chunk
            .iter()
            .map(|id| json!({"id":id,"lastEventSeq":0,"needSync":false}))
            .collect();
        let mut data = json!({"channels":channels});
        if page == 0 {
            data["manifest"] = json!(ids);
        }
        h.step(
            port_reply_with_body(
                corr,
                enveloped(
                    serde_json::to_vec(&json!({"status":"SUCCESS","data":data}))
                        .unwrap()
                        .into(),
                ),
            ),
            10 + page as u64 * 2,
        );
        assert!(!h
            .sink
            .as_slice()
            .iter()
            .any(|e| matches!(e, Effect::Http { .. } | Effect::Emit { .. })));
        let persist = h
            .sink
            .as_slice()
            .iter()
            .find_map(|e| match e {
                Effect::PersistAtomic { corr, .. } => Some(corr.raw()),
                _ => None,
            })
            .expect("page barrier");
        h.step(port_reply_ok(persist), 11 + page as u64 * 2);
        if page < 38 {
            let requests: Vec<_> = h
                .sink
                .as_slice()
                .iter()
                .filter_map(|e| match e {
                    Effect::Http { corr, req } => Some((corr.raw(), req)),
                    _ => None,
                })
                .collect();
            assert_eq!(requests.len(), 1);
            corr = requests[0].0;
            let body: serde_json::Value =
                serde_json::from_slice(requests[0].1.body.as_ref().unwrap()).unwrap();
            assert!(body["channelIds"].as_array().unwrap().len() <= 64);
        }
    }
    assert_eq!(h.module.cursor_for(channel_id(2436)), Some(0));
    assert!(
        !h.sink
            .as_slice()
            .iter()
            .any(|e| matches!(e, Effect::Emit { .. })),
        "ready must wait for final barrier"
    );
    let final_corr = h
        .sink
        .as_slice()
        .iter()
        .find_map(|e| match e {
            Effect::PersistAtomic { corr, .. } => Some(corr.raw()),
            _ => None,
        })
        .expect("final barrier");
    h.step(port_reply_ok(final_corr), 1000);
    assert!(
        h.sink
            .as_slice()
            .iter()
            .any(|e| matches!(e, Effect::Emit { .. })),
        "durable pages must reach ready"
    );
}

/// HTTP错误、超大页和非法清单均不能写入任何部分频道或启动续页。
#[test]
fn increment_pull_rejects_failed_or_invalid_pages() {
    for data in [
        json!({"manifest":["bad"],"channels":[]}),
        json!({"manifest":[cid(1),cid(1)],"channels":[]}),
        json!({"manifest":[cid(1)],"channels":[{"id":cid(2)}]}),
    ] {
        let mut h = Harness::new(&[]);
        let corr = start_pull(&mut h);
        h.step(
            port_reply_with_body(
                corr,
                enveloped(
                    serde_json::to_vec(&json!({"status":"SUCCESS","data":data}))
                        .unwrap()
                        .into(),
                ),
            ),
            3,
        );
        assert!(h.sink.as_slice().is_empty());
        assert!(h.module.cursor_for(channel_id(1)).is_none());
    }
    let mut h = Harness::new(&[]);
    let corr = start_pull(&mut h);
    h.step(port_reply_err(corr), 3);
    assert!(h.sink.as_slice().is_empty());
}

/// 断线清理HTTP关联后，旧响应不能写入下一连接。
#[test]
fn increment_pull_disconnect_discards_old_reply() {
    let mut h = Harness::new(&[]);
    let corr = start_pull(&mut h);
    h.step(
        Tick::Disconnected(helix_core::effect::TransportId::from_raw(0)),
        3,
    );
    h.step(port_reply_with_body(corr,enveloped(serde_json::to_vec(&json!({"status":"SUCCESS","data":{"manifest":[cid(1)],"channels":[{"id":cid(1)}]}})).unwrap().into())),4);
    assert!(h.sink.as_slice().is_empty());
    assert!(h.module.cursor_for(channel_id(1)).is_none());
}

/// 空成员清单也等待最终屏障，不发一个永远不会完成的恢复请求。
#[test]
fn increment_pull_empty_manifest_finishes_after_barrier() {
    let mut h = Harness::new(&[]);
    let corr = start_pull(&mut h);
    h.step(
        port_reply_with_body(
            corr,
            enveloped(
                br#"{"status":"SUCCESS","data":{"manifest":[],"channels":[]}}"#
                    .as_slice()
                    .into(),
            ),
        ),
        3,
    );
    let persist = h
        .sink
        .as_slice()
        .iter()
        .find_map(|e| match e {
            Effect::PersistAtomic { corr, .. } => Some(corr.raw()),
            _ => None,
        })
        .expect("empty final barrier");
    assert!(!h
        .sink
        .as_slice()
        .iter()
        .any(|e| matches!(e, Effect::Emit { .. })));
    h.step(port_reply_ok(persist), 4);
    assert!(h
        .sink
        .as_slice()
        .iter()
        .any(|e| matches!(e, Effect::Emit { .. })));
}
