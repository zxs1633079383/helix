//! 分批恢复公开接缝：下一页受当前页 PersistOk 背压，不能提前广播完成。
mod common;
use common::*;
use helix_core::tick::InboundBytes;
use helix_core::{Effect, Tick};
use serde_json::json;

/// 通过具有分页能力的真实形态 hello 启动恢复。
fn start_pull(h: &mut Harness) -> u64 {
    h.start(0);
    // 真启动会先扫描cursor，再扫描频道投影；回放必须兑现这些前置读取。
    let cursor = h.sink.as_slice().iter().find_map(|e| match e {
        Effect::Persist{corr,ops} if ops.iter().any(|op| matches!(op,helix_core::effect::StorageOp::Scan(spec) if spec.table=="channel_event_cursor")) => Some(corr.raw()), _=>None
    }).unwrap();
    h.step(port_reply_with_body(cursor, b"[]".as_slice().into()), 0);
    let projection = h
        .sink
        .as_slice()
        .iter()
        .find_map(|e| match e {
            Effect::Persist { corr, .. } => Some(corr.raw()),
            _ => None,
        })
        .unwrap();
    h.step(port_reply_with_body(projection, b"[]".as_slice().into()), 0);
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
    let records = std::sync::Arc::new(TimingRecords::default());
    let mut h = Harness::new_with_identity(&[], &cid(1), &cid(2), "", "", "");
    h.module = h.module.with_sync_observer(records.clone());
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
    let observations = records.0.lock().unwrap();
    use helix_im::sync_observation::{SyncResult, SyncStage};
    assert_eq!(
        observations
            .iter()
            .filter(|r| r.stage == SyncStage::InventoryPage && r.result == SyncResult::Success)
            .count(),
        39
    );
    let terminal = observations
        .iter()
        .find(|r| r.stage == SyncStage::Recovery && r.result == SyncResult::Success)
        .unwrap();
    assert_eq!(
        (terminal.total_pages, terminal.completed_at_ms),
        (Some(39), 1000)
    );
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

#[derive(Default)]
struct TimingRecords(std::sync::Mutex<Vec<helix_im::sync_observation::SyncRecord>>);
impl helix_im::sync_observation::SyncObserver for TimingRecords {
    /// 回放只捕获公开旁路记录，不驱动业务状态。
    fn record(&self, record: helix_im::sync_observation::SyncRecord) {
        self.0.lock().unwrap().push(record);
    }
}

/// 即使诊断日志关闭，空清单的整轮完成也必须等待最终持久化回执且只记录一次。
#[test]
fn sync_timing_empty_inventory_waits_for_final_persist() {
    use helix_im::sync_observation::{SyncResult, SyncStage};
    let records = std::sync::Arc::new(TimingRecords::default());
    let mut h = Harness::new_with_identity(&[], &cid(1), &cid(2), "", "", "");
    h.module = h.module.with_sync_observer(records.clone());
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
        3000,
    );
    assert!(!records
        .0
        .lock()
        .unwrap()
        .iter()
        .any(|r| r.stage == SyncStage::Recovery && r.result == SyncResult::Success));
    let persist = h
        .sink
        .as_slice()
        .iter()
        .find_map(|e| match e {
            Effect::PersistAtomic { corr, .. } => Some(corr.raw()),
            _ => None,
        })
        .unwrap();
    h.step(port_reply_ok(persist), 5000);
    h.step(port_reply_ok(persist), 6000);
    let values = records.0.lock().unwrap();
    let completed: Vec<_> = values
        .iter()
        .filter(|r| r.stage == SyncStage::Recovery && r.result == SyncResult::Success)
        .collect();
    assert_eq!(completed.len(), 1);
    assert_eq!(
        (
            completed[0].started_at_ms,
            completed[0].completed_at_ms,
            completed[0].total_pages
        ),
        (1, 5000, Some(1))
    );
    assert!(values.iter().any(|r| r.stage == SyncStage::InventoryPage
        && r.result == SyncResult::Success
        && r.completed_at_ms == 3000));
}

/// HTTP失败与断线都关闭在途页和整轮；迟到的成功不得改成恢复成功。
#[test]
fn sync_timing_failure_and_disconnect_close_once() {
    use helix_im::sync_observation::{SyncResult, SyncStage};
    for disconnected in [false, true] {
        let records = std::sync::Arc::new(TimingRecords::default());
        let mut h = Harness::new(&[]);
        h.module = h.module.with_sync_observer(records.clone());
        let corr = start_pull(&mut h);
        if disconnected {
            h.step(
                Tick::Disconnected(helix_core::effect::TransportId::from_raw(0)),
                10,
            );
        } else {
            h.step(port_reply_err(corr), 10);
        }
        h.step(port_reply_ok(corr), 20);
        let rows = records.0.lock().unwrap();
        let terminal: Vec<_> = rows
            .iter()
            .filter(|r| r.result != SyncResult::Started)
            .collect();
        assert_eq!(terminal.len(), 2);
        assert_eq!(terminal[1].stage, SyncStage::Recovery);
        assert_eq!(
            terminal[1].result,
            if disconnected {
                SyncResult::Cancelled
            } else {
                SyncResult::Failed
            }
        );
        assert_eq!(terminal[1].total_pages, None);
    }
}
