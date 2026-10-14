//! P4 SyncScheduler 全局并发窗口证伪性质测试（B4，HX-C011）。
//!
//! 回归锚: B4（257 channel 重连风暴 → 全局在途 sync ≤ K）
//!
//! 不变量（破坏即 fail，禁墙钟下界，结构化计数断言）：
//!   1. N>K channel 同时 needSync → 单次 increment_channel_end(global) 至多发 K 条 sync/notify。
//!   2. 任意时刻全局在途 sync ≤ MAX_INFLIGHT_SYNC（`sync_inflight()` 结构化读）。
//!   3. 每个 sync 回报释放 1 窗口 → drain 队首续发 1 条（不多不少，FIFO 不丢 channel）。
//!   4. 全部回报后队列清空（所有入队 channel 终被 dispatch）。
//!
//! 证伪锚点：若把 `MAX_INFLIGHT_SYNC` 调大 / 去掉队列直发 → 断言 1/2 立刻 fail（在途 > K）。
//!
//! 真源动机：现网重连 ~257 channel 同时 needSync，helix 接管 HTTP 后一次性吐 257 并发 Http →
//! 打爆驱动连接池。SyncScheduler 在 core 侧加全局窗口闸门（sync_scheduler.rs）。

mod common;
use common::*;

use helix_core::effect::Effect;
use helix_core::tick::InboundBytes;
use helix_core::Tick;
use helix_im::state::Seq;
use helix_im::sync_scheduler::MAX_INFLIGHT_SYNC;
use helix_im::sync_session::IncrementChannel;

const K: usize = MAX_INFLIGHT_SYNC;

fn inc(channel_num: u64, last_event_seq: u64) -> IncrementChannel {
    IncrementChannel {
        channel_id: channel_id(channel_num),
        last_event_seq: Seq(last_event_seq),
        need_sync: true,
        unread_post_id: None,
        last_read_seq: None,
        projection_revision: None,
        raw: bytes::Bytes::from("{}".to_string()),
    }
}

/// 收集本批 sink 中所有 sync/notify Http 的 corr.raw()（顺序 = dispatch 顺序）。
fn sync_notify_corrs(h: &Harness) -> Vec<u64> {
    h.sink
        .as_slice()
        .iter()
        .filter_map(|e| match e {
            Effect::Http { corr, req } if req.url.ends_with("/channel/sync/notify") => {
                Some(corr.raw())
            }
            _ => None,
        })
        .collect()
}

/// 驱动 storm：注册 N 个 needSync channel（cursor 种子 0），global-end 触发 SyncScheduler 入队 + drain。
fn drive_storm(h: &mut Harness, n: u64) {
    for i in 1..=n {
        h.module.ingest_increment(&inc(i, 100 + i), &mut h.sink);
    }
    h.sink.clear();
    h.module.ingest_increment_end(None, &mut h.sink);
}

// ─── ① N>K storm → 初次 fan-out 至多 K 条 sync（证伪：去窗口即 N 条）─────────────
#[test]
fn storm_initial_fanout_capped_at_k() {
    let n = (K as u64) + 12; // N 远大于 K
    let mut h = Harness::new(&[]);
    drive_storm(&mut h, n);

    let dispatched = sync_notify_corrs(&h);
    assert_eq!(
        dispatched.len(),
        K,
        "N={n}>K={K}：初次 fan-out 至多 K 条 sync/notify（破坏限流即 > K），got {} 条",
        dispatched.len()
    );
    // 结构化不变量：全局在途计数 == K（满窗），队列余 N-K。
    assert_eq!(h.module.sync_inflight(), K, "在途 sync 须 == K（满窗）");
    assert_eq!(
        h.module.sync_pending_len(),
        n as usize - K,
        "队列须余 N-K 个待 dispatch channel"
    );
}

// ─── ② 在途恒 ≤ K：逐个回报释放窗口 → 全程在途计数从不超 K（HX-C011 核心证伪）──────
#[test]
fn inflight_never_exceeds_k_through_full_drain() {
    let n = (K as u64) + 12;
    let mut h = Harness::new(&[]);
    drive_storm(&mut h, n);

    let mut inflight_corrs = sync_notify_corrs(&h);
    let mut total_dispatched = inflight_corrs.len();

    // 反复：取一个在途 sync 喂回报（释放窗口 + drain 续发），断言每步在途 ≤ K。
    let mut guard = 0;
    while !inflight_corrs.is_empty() {
        guard += 1;
        assert!(guard <= 1000, "drain 循环未收敛（疑似死锁 / 队列不清）");

        // 核心证伪：任意时刻全局在途恒 ≤ K（破坏限流即 fail）。
        assert!(
            h.module.sync_inflight() <= K,
            "在途 sync={} 超过上限 K={K}（限流被破坏）",
            h.module.sync_inflight()
        );

        let corr = inflight_corrs.remove(0);
        h.sink.clear();
        // 喂 sync 回报（no_change → 释放窗口，无续拉）。corr 经 SyncPull 路由到其 channel；
        // body 用占位 channel（999）→ parse 找不到本 channel entry → NoChange（按 corr 路由安全）。
        h.step(
            port_reply_with_body(corr, enveloped(sync_reply_no_change(999, 0))),
            2000 + guard,
        );

        // 回报后应至多新发 1 条 sync（释放 1 窗口 → drain 1）。
        let newly = sync_notify_corrs(&h);
        assert!(
            newly.len() <= 1,
            "单次回报释放 1 窗口 → 至多 drain 1 条续发，got {} 条",
            newly.len()
        );
        total_dispatched += newly.len();
        inflight_corrs.extend(newly);
    }

    // ④ 全部回报后：所有 N 个 channel 终被 dispatch，队列清空，在途归 0。
    assert_eq!(
        total_dispatched, n as usize,
        "所有 N={n} 个入队 channel 终须各 dispatch 一次（FIFO 不丢）"
    );
    assert_eq!(h.module.sync_pending_len(), 0, "全部 drain 后队列须清空");
    assert_eq!(h.module.sync_inflight(), 0, "全部回报后在途须归 0");
}

// ─── ③ N≤K：不触发限流，全部立即 dispatch（边界对照，证伪「无脑全砍到 K」误实现）────
#[test]
fn small_fanout_below_k_dispatches_all_immediately() {
    let n = (K as u64) - 2; // N < K
    let mut h = Harness::new(&[]);
    drive_storm(&mut h, n);

    let dispatched = sync_notify_corrs(&h);
    assert_eq!(
        dispatched.len(),
        n as usize,
        "N={n}≤K={K}：全部立即 dispatch（限流不应误砍未满窗的小 fan-out）"
    );
    assert_eq!(h.module.sync_pending_len(), 0, "N≤K 时队列应为空");
}

// ─── ④ 入队去重：同 channel 重复触发不双份占窗（幂等防重）────────────────────────
#[test]
fn duplicate_enqueue_does_not_double_dispatch() {
    let n = (K as u64) + 4;
    let mut h = Harness::new(&[]);
    for i in 1..=n {
        h.module.ingest_increment(&inc(i, 100 + i), &mut h.sink);
    }
    // 连发两次 global-end：第二次的 (fetched−skip) 与第一次同集 → 已在途/已在队的不重复入队。
    h.sink.clear();
    h.module.ingest_increment_end(None, &mut h.sink);
    let after_first = h.module.sync_inflight() + h.module.sync_pending_len();
    h.module.ingest_increment_end(None, &mut h.sink);
    let after_second = h.module.sync_inflight() + h.module.sync_pending_len();

    assert_eq!(
        after_first, after_second,
        "重复 global-end 不得让同 channel 双份入队（去重失效即 after_second > after_first）"
    );
    assert!(
        h.module.sync_inflight() <= K,
        "重复触发后在途仍须 ≤ K，got {}",
        h.module.sync_inflight()
    );
}

/// 回归锚：满500事件写屏障期间其他频道占满窗口，matching PersistOk 续拉必须排队。
#[test]
fn full_page_continuation_waits_for_available_sync_window() {
    let channels: Vec<u64> = (1..=(K as u64 + 1)).collect();
    let mut h = Harness::new_with_identity(&channels, "viewer-444", "tenant-1", "", "", "");
    h.step(
        command_json(
            "im_sync_channels",
            serde_json::json!({"channel_id": cid(1)}),
        ),
        1000,
    );
    let initial = sync_notify_requests(&h);
    assert_eq!(initial.len(), 1);
    assert_eq!(initial[0].1["cursors"][0]["fromSeq"], 0);
    let seqs: Vec<u64> = (1..=500).collect();
    let writes = h.step(
        port_reply_with_body(initial[0].0, enveloped(sync_reply_events(1, &seqs))),
        1500,
    );
    let persist = writes
        .iter()
        .find_map(|effect| match effect {
            EffectKind::PersistAtomic(corr) => Some(*corr),
            _ => None,
        })
        .expect("500-event HTTP reply must produce an atomic write");
    assert_eq!(h.module.cursor_for(channel_id(1)), Some(0));
    assert_eq!(h.module.sync_inflight(), 0);
    assert!(sync_notify_requests(&h).is_empty());

    let mut requests = Vec::new();
    for &channel in &channels[1..] {
        h.step(
            command_json(
                "im_sync_channels",
                serde_json::json!({"channel_id": cid(channel)}),
            ),
            1600 + channel,
        );
        let emitted = sync_notify_requests(&h);
        assert_eq!(emitted.len(), 1);
        requests.push((channel, emitted[0].0));
    }
    let mut dispatched = 1 + requests.len();
    let mut completed = 1;
    assert_eq!(dispatched - completed, K);
    assert_eq!(h.module.sync_inflight(), K);

    h.step(port_reply_ok(persist), 2000);
    let continuation = sync_notify_requests(&h);
    dispatched += continuation.len();
    eprintln!(
        "continuation_full_window: actual={} inflight={} notify={} queued={} cursor={:?}",
        dispatched - completed,
        h.module.sync_inflight(),
        continuation.len(),
        h.module.sync_pending_len(),
        h.module.cursor_for(channel_id(1))
    );
    assert_eq!(h.module.cursor_for(channel_id(1)), Some(500));
    assert_eq!(
        dispatched - completed,
        K,
        "PersistOk continuation must not emit a ninth actual request"
    );
    assert_eq!(h.module.sync_inflight(), dispatched - completed);
    assert_eq!(h.module.sync_pending_len(), 1);

    let (channel, corr) = requests[0];
    h.step(
        port_reply_with_body(corr, enveloped(sync_reply_no_change(channel, 0))),
        2100,
    );
    completed += 1;
    let resumed = sync_notify_requests(&h);
    dispatched += resumed.len();
    assert_eq!(resumed.len(), 1);
    assert_eq!(resumed[0].1["cursors"][0]["channelId"], cid(1));
    assert_eq!(resumed[0].1["cursors"][0]["fromSeq"], 500);
    assert_eq!(dispatched - completed, K);
    assert_eq!(h.module.sync_inflight(), dispatched - completed);
    assert_eq!(h.module.sync_pending_len(), 0);

    // 短页只在自身 matching PersistOk 后推进；Routine 不应变成 Pong/Hydration，也不再续拉。
    let short_page = h.step(
        port_reply_with_body(resumed[0].0, enveloped(sync_reply_events(1, &[501]))),
        2200,
    );
    let short_persist = short_page
        .iter()
        .find_map(|effect| match effect {
            EffectKind::PersistAtomic(corr) => Some(*corr),
            _ => None,
        })
        .expect("short page must produce an atomic write");
    assert_eq!(h.module.cursor_for(channel_id(1)), Some(500));
    assert!(sync_notify_requests(&h).is_empty());
    h.step(port_reply_ok(short_persist), 2300);
    assert_eq!(h.module.cursor_for(channel_id(1)), Some(501));
    assert!(sync_notify_requests(&h).is_empty());
    let state = h
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Emit { event } => {
                let payload: serde_json::Value =
                    serde_json::from_slice(event.0.as_ref()).expect("event JSON");
                (payload["event"] == "im:sync:state").then_some(payload)
            }
            _ => None,
        })
        .expect("committed routine sync state");
    assert_eq!(state["data"]["recovery"]["committedSeq"], 501);
    assert!(state["data"].get("trigger").is_none());
}

/// 从公开 step 的实际原子写 Effect 读取 correlation，不构造内部写屏障。
fn atomic_write_corr(effects: &[EffectKind]) -> u64 {
    effects
        .iter()
        .find_map(|effect| match effect {
            EffectKind::PersistAtomic(corr) => Some(*corr),
            _ => None,
        })
        .expect("public input must produce an atomic write")
}

/// 用公开命令占满其他频道窗口，返回实际请求，逐步对照 scheduler 计数。
fn fill_other_sync_window(h: &mut Harness, channels: &[u64], now_ms: u64) -> Vec<(u64, u64)> {
    let mut requests = Vec::new();
    for &channel in channels {
        h.step(
            command_json(
                "im_sync_channels",
                serde_json::json!({"channel_id": cid(channel)}),
            ),
            now_ms + channel,
        );
        let emitted = sync_notify_requests(h);
        assert_eq!(emitted.len(), 1);
        requests.push((channel, emitted[0].0));
        assert_eq!(h.module.sync_inflight(), requests.len());
        assert!(requests.len() <= K);
    }
    assert_eq!(requests.len(), K);
    requests
}

/// 真实无 action Pong 帧经公开 WS parser 进入既有 gap 优先入口。
fn pong_gap(channel: u64) -> Tick {
    Tick::Inbound(InboundBytes(
        serde_json::to_vec(&serde_json::json!({
            "status": "OK", "seq": 42,
            "data": {"gaps": [{"channelId": cid(channel), "fromSeq": 1, "maxSeq": 999}], "hashMismatch": false}
        }))
        .unwrap()
        .into(),
    ))
}

/// 回归锚：E3/B4，满窗续拉留队期间真实 WS 写提交后，dispatch 使用最新 cursor。
#[test]
fn queued_continuation_dispatches_from_latest_committed_cursor() {
    let channels: Vec<u64> = (1..=(K as u64 + 1)).collect();
    let mut h = Harness::new_with_identity(&channels, "viewer-444", "tenant-1", "", "", "");
    h.step(
        command_json(
            "im_sync_channels",
            serde_json::json!({"channel_id": cid(1)}),
        ),
        1000,
    );
    let first = sync_notify_requests(&h);
    assert_eq!(first.len(), 1);
    let full_write = atomic_write_corr(&h.step(
        port_reply_with_body(
            first[0].0,
            enveloped(sync_reply_events(1, &(1..=500).collect::<Vec<_>>())),
        ),
        1500,
    ));
    let others = fill_other_sync_window(&mut h, &channels[1..], 1600);
    let mut dispatched = 1 + others.len();
    let mut completed = 1;
    h.step(port_reply_ok(full_write), 2000);
    dispatched += sync_notify_requests(&h).len();
    assert_eq!(dispatched - completed, K);
    assert_eq!(h.module.sync_inflight(), dispatched - completed);
    assert_eq!(h.module.sync_pending_len(), 1);
    assert_eq!(h.module.cursor_for(channel_id(1)), Some(500));

    // pending_stream_seq 是实时写占位；真实 matching 回执推进 cursor，仍不应抢第九窗口。
    let ws_write = atomic_write_corr(&h.step(inbound_event(1, 501, 1), 2100));
    assert_eq!(h.module.cursor_for(channel_id(1)), Some(500));
    assert!(sync_notify_requests(&h).is_empty());
    h.step(port_reply_ok(ws_write), 2200);
    assert_eq!(h.module.cursor_for(channel_id(1)), Some(501));
    assert!(sync_notify_requests(&h).is_empty());
    assert_eq!(h.module.sync_pending_len(), 1);
    assert_eq!(h.module.sync_inflight(), dispatched - completed);

    let (channel, corr) = others[0];
    h.step(
        port_reply_with_body(corr, enveloped(sync_reply_no_change(channel, 0))),
        2300,
    );
    completed += 1;
    let resumed = sync_notify_requests(&h);
    dispatched += resumed.len();
    assert_eq!(resumed.len(), 1);
    assert_eq!(resumed[0].1["cursors"][0]["channelId"], cid(1));
    assert_eq!(resumed[0].1["cursors"][0]["fromSeq"], 501);
    assert_eq!(dispatched - completed, K);
    assert_eq!(h.module.sync_inflight(), dispatched - completed);
    assert_eq!(h.module.sync_pending_len(), 0);
    eprintln!(
        "queued_latest_cursor: from_seq={} cursor={:?} actual={} inflight={}",
        resumed[0].1["cursors"][0]["fromSeq"],
        h.module.cursor_for(channel_id(1)),
        dispatched - completed,
        h.module.sync_inflight()
    );

    let short_write = atomic_write_corr(&h.step(
        port_reply_with_body(resumed[0].0, enveloped(sync_reply_events(1, &[502]))),
        2400,
    ));
    completed += 1;
    assert_eq!(h.module.cursor_for(channel_id(1)), Some(501));
    assert!(sync_notify_requests(&h).is_empty());
    assert_eq!(h.module.sync_inflight(), dispatched - completed);
    h.step(port_reply_ok(short_write), 2500);
    assert_eq!(h.module.cursor_for(channel_id(1)), Some(502));
    assert!(
        sync_notify_requests(&h).is_empty(),
        "short page must end continuation"
    );
    assert_eq!(h.module.sync_pending_len(), 0);
}

/// 回归锚：E3/UC-4.4，两种真实入口顺序均保留 Pong 升级，满页确认不越八窗口。
#[test]
fn queued_full_page_preserves_pong_trigger_in_both_orders() {
    for initial_pong in [false, true] {
        let channels: Vec<u64> = (1..=(K as u64 + 1)).collect();
        let mut h = Harness::new_with_identity(&channels, "viewer-444", "tenant-1", "", "", "");
        let routine = || {
            command_json(
                "im_sync_channels",
                serde_json::json!({"channel_id": cid(1)}),
            )
        };
        h.step(if initial_pong { pong_gap(1) } else { routine() }, 1000);
        let first = sync_notify_requests(&h);
        assert_eq!(first.len(), 1);
        let full_write = atomic_write_corr(&h.step(
            port_reply_with_body(
                first[0].0,
                enveloped(sync_reply_events(1, &(1..=500).collect::<Vec<_>>())),
            ),
            1500,
        ));
        // Routine 满页 + queued Pong，以及 Pong 满页 + queued Routine，均须等待自身写回执。
        h.step(if initial_pong { routine() } else { pong_gap(1) }, 1600);
        assert!(sync_notify_requests(&h).is_empty());
        assert_eq!(h.module.cursor_for(channel_id(1)), Some(0));
        assert_eq!(h.module.sync_pending_len(), 1);
        let others = fill_other_sync_window(&mut h, &channels[1..], 1700);
        let mut dispatched = 1 + others.len();
        let mut completed = 1;
        h.step(port_reply_ok(full_write), 2000);
        dispatched += sync_notify_requests(&h).len();
        assert_eq!(
            dispatched - completed,
            K,
            "initial_pong={initial_pong}: no ninth request"
        );
        assert_eq!(h.module.sync_inflight(), dispatched - completed);
        assert_eq!(h.module.sync_pending_len(), 1);
        assert_eq!(h.module.cursor_for(channel_id(1)), Some(500));

        let (channel, corr) = others[0];
        h.step(
            port_reply_with_body(corr, enveloped(sync_reply_no_change(channel, 0))),
            2100,
        );
        completed += 1;
        let resumed = sync_notify_requests(&h);
        dispatched += resumed.len();
        assert_eq!(resumed.len(), 1);
        assert_eq!(resumed[0].1["cursors"][0]["channelId"], cid(1));
        assert_eq!(resumed[0].1["cursors"][0]["fromSeq"], 500);
        assert_eq!(dispatched - completed, K);
        assert_eq!(h.module.sync_inflight(), dispatched - completed);
        assert_eq!(h.module.sync_pending_len(), 0);

        let short_write = atomic_write_corr(&h.step(
            port_reply_with_body(resumed[0].0, enveloped(sync_reply_events(1, &[501]))),
            2200,
        ));
        completed += 1;
        assert_eq!(h.module.cursor_for(channel_id(1)), Some(500));
        assert!(sync_notify_requests(&h).is_empty());
        assert_eq!(h.module.sync_inflight(), dispatched - completed);
        h.step(port_reply_ok(short_write), 2300);
        assert_eq!(h.module.cursor_for(channel_id(1)), Some(501));
        assert!(sync_notify_requests(&h).is_empty());
        let state = h
            .sink
            .as_slice()
            .iter()
            .find_map(|effect| match effect {
                Effect::Emit { event } => {
                    let payload: serde_json::Value =
                        serde_json::from_slice(event.0.as_ref()).expect("event JSON");
                    (payload["event"] == "im:sync:state").then_some(payload)
                }
                _ => None,
            })
            .expect("committed resumed sync state");
        assert_eq!(
            state["data"]["trigger"], "pong_gap",
            "initial_pong={initial_pong}"
        );
        assert_eq!(state["data"]["recovery"]["committedSeq"], 501);
        eprintln!("queued_trigger_order: initial_pong={initial_pong} trigger={} cursor={:?} actual={} inflight={}",
            state["data"]["trigger"], h.module.cursor_for(channel_id(1)), dispatched - completed, h.module.sync_inflight());
    }
}
