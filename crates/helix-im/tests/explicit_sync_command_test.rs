mod common;

use common::{
    channel_id, cid, enveloped, hello_inbound, port_reply_ok, port_reply_with_body,
    sync_reply_events, Harness,
};
use helix_core::tick::{InboundBytes, PortError, PortOutcome};
use helix_core::Correlation;
use helix_core::{Effect, Module, Tick};

fn sync_command(channel_id: &str) -> Tick {
    Tick::Command(
        helix_im::client_api::build_command(
            "im_sync_channels",
            serde_json::json!({
                "channelId": channel_id,
                "reason": "offline_revoke_replay",
                "eventSeq": 999_999
            })
            .to_string()
            .as_bytes(),
        )
        .expect("public sync command must normalize"),
    )
}

/// 只数公开入口生成的 notify，command ACK 不能替代 HTTP 请求证据。
fn notify_requests(harness: &Harness) -> Vec<(u64, serde_json::Value)> {
    harness
        .sink
        .as_slice()
        .iter()
        .filter_map(|effect| match effect {
            Effect::Http { corr, req } if req.url.ends_with("/channel/sync/notify") => {
                let body = serde_json::from_slice(req.body.as_ref().unwrap().as_ref()).unwrap();
                Some((corr.raw(), body))
            }
            _ => None,
        })
        .collect()
}

/// 使用真实存储错误分类回灌 matching 或未知 corr。
fn persist_error(corr_raw: u64) -> Tick {
    Tick::PortReply {
        corr: Correlation::from_raw(corr_raw),
        outcome: PortOutcome::Err(PortError::Storage(1)),
    }
}

/// 读取真实公开事件出口，以核对 Pong trigger 和排空终态。
fn emitted_event(harness: &Harness, name: &str) -> Vec<serde_json::Value> {
    harness
        .sink
        .as_slice()
        .iter()
        .filter_map(|effect| match effect {
            Effect::Emit { event } => {
                let value: serde_json::Value = serde_json::from_slice(event.0.as_ref()).unwrap();
                (value["event"] == name).then_some(value)
            }
            _ => None,
        })
        .collect()
}

#[test]
fn public_sync_command_uses_helix_cursor_and_sync_notify_not_hydration() {
    let mut harness = Harness::new(&[]);
    harness.module.register_channel(channel_id(5), 41);
    harness.step(hello_inbound("conn-explicit-sync"), 1_000);
    let command = sync_command(&cid(5));
    assert!(harness.module.accepts(&command));

    harness.step(command, 1_001);

    let (url, body) = harness
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Http { req, .. } => Some((req.url.as_str(), req.body.as_ref().unwrap())),
            _ => None,
        })
        .expect("explicit sync must emit one HTTP effect");
    assert!(url.ends_with("/channel/sync/notify"), "url={url}");
    assert!(!url.contains("incrementByChannelId"));
    let body: serde_json::Value = serde_json::from_slice(body.as_ref()).unwrap();
    assert_eq!(body["cursors"][0]["channelId"], cid(5));
    assert_eq!(body["cursors"][0]["fromSeq"], 41);
    assert_ne!(body["cursors"][0]["fromSeq"], 999_999);
}

#[test]
fn explicit_sync_registers_unknown_channel_at_zero_for_authoritative_pull() {
    let mut harness = Harness::new(&[]);
    harness.step(hello_inbound("conn-explicit-unknown"), 1_000);

    harness.step(sync_command(&cid(6)), 1_001);

    let body = harness.last_http_body().expect("sync HTTP body");
    assert_eq!(body["cursors"][0]["channelId"], cid(6));
    assert_eq!(body["cursors"][0]["fromSeq"], 0);
    assert_eq!(harness.module.cursor_for(channel_id(6)), Some(0));
}

// 回归锚: B4，同频道同步守卫必须覆盖 HTTP 回执至 matching PersistOk 的写屏障。
#[test]
fn public_sync_waits_for_matching_persist_without_blocking_other_channels() {
    let mut harness = Harness::new_with_identity(&[5, 6], "viewer-444", "tenant-1", "", "", "");
    harness.step(hello_inbound("conn-explicit-persist-gap"), 1_000);

    harness.step(sync_command(&cid(5)), 1_001);
    let first = notify_requests(&harness);
    assert_eq!(first.len(), 1, "A 必须产生一个实际 notify effect");
    assert_eq!(first[0].1["cursors"][0]["channelId"], cid(5));
    assert_eq!(first[0].1["cursors"][0]["fromSeq"], 0);

    harness.step(
        port_reply_with_body(first[0].0, enveloped(sync_reply_events(5, &[1, 2, 3]))),
        1_002,
    );
    let persist_corr = harness
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::PersistAtomic { corr, .. } => Some(corr.raw()),
            _ => None,
        })
        .expect("普通 Events 必须进入 correlated PersistAtomic");
    assert_ne!(persist_corr, first[0].0, "HTTP 与存储必须各有匹配 corr");
    assert_eq!(harness.module.cursor_for(channel_id(5)), Some(0));

    // 暂留 A 的 matching PersistOk；它的写屏障不能阻塞无关频道。
    harness.step(sync_command(&cid(6)), 1_003);
    let other = notify_requests(&harness);
    assert_eq!(other.len(), 1, "其他频道在 A 待提交时仍应可同步");
    assert_eq!(other[0].1["cursors"][0]["channelId"], cid(6));
    assert_eq!(other[0].1["cursors"][0]["fromSeq"], 0);

    harness.step(sync_command(&cid(5)), 1_004);
    let repeated = notify_requests(&harness);
    let same_channel_requests = repeated
        .iter()
        .filter(|(_, body)| body["cursors"][0]["channelId"] == cid(5))
        .count();
    assert_eq!(
        same_channel_requests, 0,
        "matching PersistOk 尚未到达时，同频道不得再次 notify 旧 cursor；persist_corr={persist_corr}, requests={repeated:?}"
    );

    // 未知成功/失败回执都不能释放 A 的真实写屏障。
    for tick in [port_reply_ok(999_998), persist_error(999_999)] {
        harness.step(tick, 1_005);
        harness.step(sync_command(&cid(5)), 1_006);
        assert!(notify_requests(&harness).is_empty());
        assert_eq!(harness.module.cursor_for(channel_id(5)), Some(0));
    }

    // Pong 优先调度也必须沿用同一个写屏障，不能绕回旧 cursor。
    let pong = serde_json::json!({
        "status": "OK", "seq": 42,
        "data": {"gaps": [{"channelId": cid(5), "fromSeq": 1, "maxSeq": 9}], "hashMismatch": false}
    });
    harness.step(
        Tick::Inbound(InboundBytes(serde_json::to_vec(&pong).unwrap().into())),
        6_001,
    );
    assert!(notify_requests(&harness).is_empty());

    harness.step(port_reply_ok(persist_corr), 6_002);
    assert_eq!(harness.module.cursor_for(channel_id(5)), Some(3));
    let committed = notify_requests(&harness);
    assert_eq!(committed.len(), 1, "matching 成功后必须允许新同步");
    assert_eq!(committed[0].1["cursors"][0]["channelId"], cid(5));
    assert_eq!(committed[0].1["cursors"][0]["fromSeq"], 3);
    harness.step(sync_command(&cid(5)), 6_003);
    assert!(
        notify_requests(&harness).is_empty(),
        "已自动补发，不得重复请求"
    );
}

// 回归锚: B4，matching 持久失败只能释放自己的屏障，并允许原 cursor 重试。
#[test]
fn public_sync_retries_unchanged_cursor_after_matching_persist_error() {
    let mut harness = Harness::new_with_identity(&[5, 6], "viewer-444", "tenant-1", "", "", "");
    harness.step(hello_inbound("conn-explicit-persist-error"), 1_000);
    harness.step(sync_command(&cid(5)), 1_001);
    let first = notify_requests(&harness);
    assert_eq!(first.len(), 1);
    harness.step(
        port_reply_with_body(first[0].0, enveloped(sync_reply_events(5, &[1, 2, 3]))),
        1_002,
    );
    let persist_corr = harness
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::PersistAtomic { corr, .. } => Some(corr.raw()),
            _ => None,
        })
        .expect("普通 Events 必须产生 matching 原子写");

    // B 也进入待提交态；A 失败不能释放 B 的屏障。
    harness.step(sync_command(&cid(6)), 1_003);
    let other = notify_requests(&harness);
    assert_eq!(other.len(), 1);
    assert_eq!(other[0].1["cursors"][0]["channelId"], cid(6));
    harness.step(
        port_reply_with_body(other[0].0, enveloped(sync_reply_events(6, &[1]))),
        1_004,
    );
    assert!(harness
        .sink
        .as_slice()
        .iter()
        .any(|effect| matches!(effect, Effect::PersistAtomic { .. })));

    harness.step(persist_error(persist_corr), 1_005);
    assert_eq!(harness.module.cursor_for(channel_id(5)), Some(0));
    harness.step(sync_command(&cid(6)), 1_006);
    assert!(notify_requests(&harness).is_empty(), "A 失败不能释放 B");
    harness.step(sync_command(&cid(5)), 1_007);
    let retry = notify_requests(&harness);
    assert_eq!(retry.len(), 1, "matching 失败后必须允许 retry");
    assert_eq!(retry[0].1["cursors"][0]["channelId"], cid(5));
    assert_eq!(retry[0].1["cursors"][0]["fromSeq"], 0);
}

// 回归锚: B4，普通短批不会自动分页，新意图必须留到 matching PersistOk 后兑现。
#[test]
fn pending_persist_retains_explicit_and_pong_intents_until_committed_cursor() {
    let mut failures = Vec::new();
    for source in ["explicit", "pong"] {
        let mut harness = Harness::new_with_identity(&[5, 6], "viewer-444", "tenant-1", "", "", "");
        harness.step(hello_inbound("conn-deferred-sync-intent"), 1_000);
        harness.step(sync_command(&cid(5)), 1_001);
        let first = notify_requests(&harness);
        assert_eq!(first.len(), 1);
        // 3 条普通 Events 小于 wire 满批 500，真实 parser 的 wants_continuation=false。
        harness.step(
            port_reply_with_body(first[0].0, enveloped(sync_reply_events(5, &[1, 2, 3]))),
            1_002,
        );
        let persist_corr = harness
            .sink
            .as_slice()
            .iter()
            .find_map(|effect| match effect {
                Effect::PersistAtomic { corr, .. } => Some(corr.raw()),
                _ => None,
            })
            .expect("短批必须进入真实持久化屏障");

        let intent = if source == "explicit" {
            sync_command(&cid(5))
        } else {
            Tick::Inbound(InboundBytes(
                serde_json::to_vec(&serde_json::json!({
                    "status": "OK", "seq": 42,
                    "data": {"gaps": [{"channelId": cid(5), "fromSeq": 4, "maxSeq": 9}], "hashMismatch": false}
                }))
                .unwrap()
                .into(),
            ))
        };
        harness.step(intent, 6_001);
        assert!(
            notify_requests(&harness).is_empty(),
            "{source}: 待 Persist 期间不得拉旧 cursor"
        );
        // 重复显式意图也只能合并为一个；Pong 优先项不能被 Routine 覆盖。
        harness.step(sync_command(&cid(5)), 6_002);
        assert!(notify_requests(&harness).is_empty());
        harness.step(sync_command(&cid(6)), 6_003);
        let other = notify_requests(&harness);
        assert_eq!(other.len(), 1, "{source}: 队首被屏障阻挡不能堵无关频道");
        assert_eq!(other[0].1["cursors"][0]["channelId"], cid(6));

        harness.step(port_reply_ok(persist_corr), 6_004);
        assert_eq!(harness.module.cursor_for(channel_id(5)), Some(3));
        let resumed = notify_requests(&harness);
        if resumed.len() != 1
            || resumed[0].1["cursors"][0]["channelId"] != cid(5)
            || resumed[0].1["cursors"][0]["fromSeq"] != 3
        {
            failures.push(format!(
                "{source}: matching PersistOk 后无需新 command，应恰好一次 from3；actual={resumed:?}"
            ));
        }
        harness.step(port_reply_ok(persist_corr), 6_005);
        assert!(notify_requests(&harness).is_empty(), "迟到重复回执不能重发");
        if resumed.len() == 1 {
            harness.step(
                port_reply_with_body(resumed[0].0, enveloped(sync_reply_events(5, &[4]))),
                6_006,
            );
            let resumed_persist = harness
                .sink
                .as_slice()
                .iter()
                .find_map(|effect| match effect {
                    Effect::PersistAtomic { corr, .. } => Some(corr.raw()),
                    _ => None,
                })
                .expect("续发仍必须完成实际写屏障");
            harness.step(port_reply_ok(resumed_persist), 6_007);
            assert_eq!(harness.module.cursor_for(channel_id(5)), Some(4));
            let states = emitted_event(&harness, "im:sync:state");
            assert_eq!(states.len(), 1);
            if source == "pong" {
                assert_eq!(states[0]["data"]["trigger"], "pong_gap");
            } else {
                assert!(states[0]["data"].get("trigger").is_none());
            }
            assert!(
                emitted_event(&harness, "im:sync:gap-repaired").is_empty(),
                "无关频道仍在途，不能提前宣布全批排空"
            );
            harness.step(
                port_reply_with_body(other[0].0, enveloped(sync_reply_events(6, &[]))),
                6_008,
            );
            let completed = emitted_event(&harness, "im:sync:gap-repaired");
            assert_eq!(completed.len(), usize::from(source == "pong"));
            harness.step(port_reply_ok(resumed_persist), 6_009);
            assert!(emitted_event(&harness, "im:sync:gap-repaired").is_empty());
        }
    }

    // 独立旧机制边界：HTTP 尚在途时的命令一直被合并到原请求，不属于新增 Persist 窗口。
    let mut inflight = Harness::new(&[5]);
    inflight.step(hello_inbound("conn-existing-http-inflight"), 1_000);
    inflight.step(sync_command(&cid(5)), 1_001);
    let first = notify_requests(&inflight);
    assert_eq!(first.len(), 1);
    inflight.step(sync_command(&cid(5)), 1_002);
    assert!(notify_requests(&inflight).is_empty());
    inflight.step(
        port_reply_with_body(first[0].0, enveloped(sync_reply_events(5, &[]))),
        1_003,
    );
    assert!(
        notify_requests(&inflight).is_empty(),
        "原 HTTP-inflight 短批仍不新增自动分页；与新持久窗口分开"
    );
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

// 回归锚: B4，未知回执不释放屏障，matching Err 兑现留队意图但不推进 cursor。
#[test]
fn pending_intent_retries_only_matching_persist_error_without_advancing_cursor() {
    let mut harness = Harness::new_with_identity(&[5], "viewer-444", "tenant-1", "", "", "");
    harness.step(hello_inbound("conn-deferred-intent-error"), 1_000);
    harness.step(sync_command(&cid(5)), 1_001);
    let first = notify_requests(&harness);
    harness.step(
        port_reply_with_body(first[0].0, enveloped(sync_reply_events(5, &[1, 2, 3]))),
        1_002,
    );
    let persist_corr = harness
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::PersistAtomic { corr, .. } => Some(corr.raw()),
            _ => None,
        })
        .unwrap();
    harness.step(sync_command(&cid(5)), 1_003);
    assert!(notify_requests(&harness).is_empty());
    for tick in [persist_error(999_998), port_reply_ok(999_999)] {
        harness.step(tick, 1_004);
        assert!(notify_requests(&harness).is_empty());
        assert_eq!(harness.module.cursor_for(channel_id(5)), Some(0));
    }
    harness.step(persist_error(persist_corr), 1_005);
    assert_eq!(harness.module.cursor_for(channel_id(5)), Some(0));
    let retry = notify_requests(&harness);
    assert_eq!(retry.len(), 1, "matching Err 自动兑现已留队意图");
    assert_eq!(retry[0].1["cursors"][0]["fromSeq"], 0);
    harness.step(persist_error(persist_corr), 1_006);
    assert!(notify_requests(&harness).is_empty());
}

// 回归锚: B4，被写屏障阻挡的 Pong 仍优先于 Routine，且不能突破全局窗口。
#[test]
fn blocked_pong_intent_keeps_priority_ahead_of_full_window_backlog() {
    let mut harness = Harness::new_with_identity(
        &(5..=15).collect::<Vec<_>>(),
        "viewer-444",
        "tenant-1",
        "",
        "",
        "",
    );
    harness.step(hello_inbound("conn-deferred-pong-priority"), 1_000);
    harness.step(sync_command(&cid(5)), 1_001);
    let first = notify_requests(&harness);
    harness.step(
        port_reply_with_body(first[0].0, enveloped(sync_reply_events(5, &[1, 2, 3]))),
        1_002,
    );
    let persist_corr = harness
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::PersistAtomic { corr, .. } => Some(corr.raw()),
            _ => None,
        })
        .unwrap();
    harness.step(
        Tick::Inbound(InboundBytes(
            serde_json::to_vec(&serde_json::json!({
                "status": "OK", "seq": 42,
                "data": {"gaps": [{"channelId": cid(5), "fromSeq": 4, "maxSeq": 9}], "hashMismatch": false}
            }))
            .unwrap()
            .into(),
        )),
        6_001,
    );
    assert!(notify_requests(&harness).is_empty());
    let mut first_other = None;
    for id in 6..=13 {
        harness.step(sync_command(&cid(id)), 6_002);
        let requests = notify_requests(&harness);
        assert_eq!(requests.len(), 1, "被阻挡队首不得阻塞可用窗口");
        if id == 6 {
            first_other = Some(requests[0].0);
        }
    }
    harness.step(sync_command(&cid(14)), 6_003);
    assert!(notify_requests(&harness).is_empty(), "全局 8 窗口上限");
    harness.step(port_reply_ok(persist_corr), 6_004);
    assert!(
        notify_requests(&harness).is_empty(),
        "提交成功也不能超全局窗口"
    );
    harness.step(
        port_reply_with_body(first_other.unwrap(), enveloped(sync_reply_events(6, &[]))),
        6_005,
    );
    let resumed = notify_requests(&harness);
    assert_eq!(resumed.len(), 1);
    assert_eq!(resumed[0].1["cursors"][0]["channelId"], cid(5));
    assert_eq!(resumed[0].1["cursors"][0]["fromSeq"], 3);
}
