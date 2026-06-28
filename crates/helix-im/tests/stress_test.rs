//! 洪峰压测：内核步进吞吐（sans-IO 纯状态机，无 driver / 无 I/O）
//!
//! ## 测什么
//!
//! 1. `stress_inorder_100k`：注册 1 个 channel，喂 10 万条连续 seq 的
//!    Tick::Inbound（posted 帧 JSON），测 ImModule::handle 的 step 吞吐（帧/秒）。
//!    每帧产出 3 个 Effect（PersistFire 消息 + PersistFire cursor + Emit），
//!    度量包含 JSON parse + Effect 构造（含 String 分配），不含 driver 兑现。
//! 2. `stress_gap_5000_ws_flush`：乱序场景 A —— 5000 条带缺口事件堆入 buffer
//!    （arm gate），随后补上缺口 seq=1，单次 step 触发 5000 条 contiguous flush。
//! 3. `stress_gap_5000_sync_backfill`：乱序场景 B —— gate timer 超时 →
//!    Http{sync/notify} → PortReply 灌入 5000 events 的 sync 响应
//!    （parse + 5000 个 BatchUpsert op 构造），再 PortReply Ok 触发
//!    commit + buffer flush。
//!
//! ## 跑法
//!
//! ```bash
//! cargo test --release -p helix-im stress -- --nocapture --test-threads=1
//! ```
//!
//! 注意：workspace release profile 是 opt-level="z" + lto（面向 wasm 体积），
//! 不是 opt-level=3 —— 数字是保守值（生产 native 端只会更快）。

// 测量类压测文件：Instant 计时与 --nocapture 打印是用途本身，非业务代码（clippy.toml deny 的显式豁免）
#![allow(clippy::disallowed_methods, clippy::disallowed_macros)]

use std::time::Instant;

use helix_core::tick::{InboundBytes, PortOutcome, ReplyBytes};
use helix_core::{Effect, EffectSink, Module, Tick};
use helix_im::module::ImConfig;
use helix_im::ImModule;

mod common;
use common::channel_id;

// ─── 辅助 ────────────────────────────────────────────────────────────────────

fn make_module(channel_num: u64) -> ImModule {
    let config = ImConfig {
        ws_url: "ws://stress".to_string(),
        api_base_url: "http://stress".to_string(),
        default_api_base_url: "http://localhost:3399".to_string(),
        ping_interval_ms: 8_000,
        send_timeout_ms: 15_000,
        auth_user_id: String::new(),
        company_id: String::new(),
        user_name: String::new(),
        org_name: String::new(),
        dept_name: String::new(),
    };
    let mut module = ImModule::new(config);
    module.register_channel(channel_id(channel_num), 0);
    module
}

/// 测试桥接：数值 channel id → 真 wire 26 字符字符串。
fn cid(n: u64) -> String {
    channel_id(n).as_str().to_string()
}

/// 标准 base64 编码（与 host base64_encode 同表）。dev-zlc ADR-007 sync 信封 body 用。
fn base64_std(bytes: &[u8]) -> String {
    const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(TABLE[((n >> 18) & 63) as usize] as char);
        out.push(TABLE[((n >> 12) & 63) as usize] as char);
        out.push(if chunk.len() > 1 {
            TABLE[((n >> 6) & 63) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            TABLE[(n & 63) as usize] as char
        } else {
            '='
        });
    }
    out
}

/// 构造 posted WS 帧（与 common.rs::inbound_event 同构，payload 约 150 字节）
fn make_inbound_tick(channel_id: u64, seq: u64) -> Tick {
    let payload = serde_json::json!({
        "event": "posted",
        "data": {
            "channel_id": cid(channel_id),
            "seq": seq,
            "event_type": 1,
            "msg_id": format!("msg_{}", seq),
            "actor_id": "user_1",
            "created_at": 1000000_u64,
            "payload": null
        }
    });
    let bytes = serde_json::to_vec(&payload).expect("build inbound json");
    Tick::Inbound(InboundBytes(bytes.into()))
}

// ─── 测试 1：10 万条连续 seq 的 step 吞吐 ────────────────────────────────────

#[test]
fn stress_inorder_100k() {
    const N: u64 = 100_000;
    let mut module = make_module(1);
    let mut sink = EffectSink::new();

    // 预构建全部 Tick（构造成本不计入步进度量）
    let ticks: Vec<Tick> = (1..=N).map(|s| make_inbound_tick(1, s)).collect();
    let avg_frame_len: usize = ticks
        .iter()
        .map(|t| match t {
            Tick::Inbound(b) => b.as_bytes().len(),
            _ => 0,
        })
        .sum::<usize>()
        / N as usize;

    let mut total_effects: u64 = 0;
    let start = Instant::now();
    for tick in &ticks {
        sink.clear();
        module
            .handle(tick, 1_000_000, &mut sink)
            .expect("handle inorder");
        total_effects += sink.as_slice().len() as u64;
    }
    let elapsed = start.elapsed();

    assert_eq!(
        module.cursor_for(channel_id(1)),
        Some(N),
        "cursor must reach N"
    );
    assert_eq!(
        total_effects,
        N * 3,
        "each in-order frame must yield 3 effects"
    );

    let fps = N as f64 / elapsed.as_secs_f64();
    eprintln!("\n[stress_inorder_100k]");
    eprintln!("  frames           : {}", N);
    eprintln!("  avg frame size   : {} bytes (JSON)", avg_frame_len);
    eprintln!("  total time       : {:.3} ms", elapsed.as_secs_f64() * 1e3);
    eprintln!("  step throughput  : {:.0} frames/s", fps);
    eprintln!(
        "  per-frame latency: {:.3} us",
        elapsed.as_secs_f64() * 1e6 / N as f64
    );
    eprintln!("  effects produced : {}", total_effects);
}

// ─── 测试 2：5000 缺口 buffer 堆积 + WS 补洞一次性 flush ─────────────────────

#[test]
fn stress_gap_5000_ws_flush() {
    const GAP: u64 = 5_000;
    let mut module = make_module(1);
    let mut sink = EffectSink::new();

    // 乱序：seq 2..=5001（缺 seq=1），全部进 buffer（BTreeMap 堆积）
    let buffered_ticks: Vec<Tick> = (2..=GAP + 1).map(|s| make_inbound_tick(1, s)).collect();

    let start_buf = Instant::now();
    for tick in &buffered_ticks {
        sink.clear();
        module
            .handle(tick, 1_000_000, &mut sink)
            .expect("handle gap event");
    }
    let buf_elapsed = start_buf.elapsed();
    assert_eq!(
        module.cursor_for(channel_id(1)),
        Some(0),
        "cursor must NOT advance while gapped"
    );

    // 补上缺口 seq=1：单次 step 触发 apply + flush_contiguous 5000 条
    let hole_tick = make_inbound_tick(1, 1);
    sink.clear();
    let start_flush = Instant::now();
    module
        .handle(&hole_tick, 1_000_001, &mut sink)
        .expect("handle hole fill");
    let flush_elapsed = start_flush.elapsed();

    assert_eq!(
        module.cursor_for(channel_id(1)),
        Some(GAP + 1),
        "cursor must reach 5001"
    );
    // 5001 个事件 × 3 effects（首条的 gate 已 arm，flush 不再 arm timer）
    let flush_effects = sink.as_slice().len();
    assert!(
        flush_effects >= (GAP as usize + 1) * 3,
        "flush step must yield >= {} effects, got {}",
        (GAP + 1) * 3,
        flush_effects
    );

    let buf_rate = GAP as f64 / buf_elapsed.as_secs_f64();
    eprintln!("\n[stress_gap_5000_ws_flush]");
    eprintln!(
        "  buffering phase  : {} frames in {:.3} ms ({:.0} frames/s)",
        GAP,
        buf_elapsed.as_secs_f64() * 1e3,
        buf_rate
    );
    eprintln!(
        "  hole-fill flush  : 1 step flushed {} events in {:.3} ms",
        GAP + 1,
        flush_elapsed.as_secs_f64() * 1e3
    );
    eprintln!(
        "  flush throughput : {:.0} events/s",
        (GAP + 1) as f64 / flush_elapsed.as_secs_f64()
    );
    eprintln!("  flush effects    : {}", flush_effects);
}

// ─── 测试 3：5000 缺口 sync backfill（gate → sync/notify → 批量落库 → commit）─

#[test]
fn stress_gap_5000_sync_backfill() {
    const GAP: u64 = 5_000;
    let mut module = make_module(1);
    let mut sink = EffectSink::new();

    // 1. 喂一条远端 seq=5001（缺口 1..=5000）→ buffer + **立即** backfill resync（根治
    //    fix/im-sync-cursor-catchup·gap=5000 ≫ 阈值）：sync/notify Http 在 ingest 当步即发出，
    //    不再等 1s gate timer fire（gate timer 仍 arm 作兜底，但 cursor 由 immediate sync 追平）。
    let far_tick = make_inbound_tick(1, GAP + 1);
    sink.clear();
    module
        .handle(&far_tick, 1_000_000, &mut sink)
        .expect("handle far event");
    let sync_corr = sink
        .as_slice()
        .iter()
        .find_map(|e| match e {
            Effect::Http { corr, .. } => Some(*corr),
            _ => None,
        })
        .expect("大缺口 ingest 当步即发 sync/notify Http（立即 backfill）");

    // 3. 构造 5000 events 的 sync 响应（~600KB JSON），PortReply 灌入
    //    度量：JSON parse + 分桶排序 + 5000 个 BatchUpsert op 构造
    //    C2：内容在分离的 messages map（key=msgId）。本压测全可见（无 phantom）——
    //    每个 event 的 msgId 都在 messages map，落 5000 条真 upsert（热路径压测意图保留）。
    let events: Vec<serde_json::Value> = (1..=GAP)
        .map(|s| {
            serde_json::json!({
                "channelId": cid(1),
                "eventSeq": s,
                "eventType": 1,
                "msgId": format!("msg_{}", s),
                "actorId": "user_1",
                "createAt": 1000000_u64,
            })
        })
        .collect();
    // messages map：每个 msgId → 内容对象（含 read_bits 空位）。全可见 = 0 phantom。
    let mut messages = serde_json::Map::with_capacity(GAP as usize);
    for s in 1..=GAP {
        let mid = format!("msg_{}", s);
        messages.insert(
            mid.clone(),
            serde_json::json!({
                "id": mid,
                "temporary_id": mid,
                "channel_id": cid(1),
                "message": "stress body",
                "readBits": "",
            }),
        );
    }
    // 真 Go wire：{status:SUCCESS, data:{entries:[SyncEntry], serverTime}}。
    // 单 entry 5000 events（超 500 上限 → parser 派生续拉，对本压测断言无影响）。
    let resp = serde_json::json!({
        "status": "SUCCESS",
        "data": {
            "entries": [{
                "channelId": cid(1),
                "kind": "events",
                "events": events,
                "messages": serde_json::Value::Object(messages),
                "nextSeq": GAP,
                "resetTo": serde_json::Value::Null,
            }],
            "serverTime": 1_700_000_000_i64
        }
    });
    let resp_bytes = serde_json::to_vec(&resp).expect("build sync response");
    let resp_len = resp_bytes.len();
    // dev-zlc：sync 回报走 ADR-007 信封 {status,headers,body:base64(裸 Go body)}，
    // handle_sync_reply 解信封 + base64 decode 后才喂 parse_sync_response。
    let envelope = serde_json::json!({
        "status": 200,
        "headers": {},
        "body": base64_std(&resp_bytes),
    });
    let envelope_bytes = serde_json::to_vec(&envelope).expect("build sync envelope");
    let reply_tick = Tick::PortReply {
        corr: sync_corr,
        outcome: PortOutcome::Ok(ReplyBytes(envelope_bytes.into())),
    };

    sink.clear();
    let start_parse = Instant::now();
    module
        .handle(&reply_tick, 1_001_100, &mut sink)
        .expect("sync reply");
    let parse_elapsed = start_parse.elapsed();
    let persist_corr = sink
        .as_slice()
        .iter()
        .find_map(|e| match e {
            Effect::Persist { corr, ops } => {
                // A3/CAP-9：全 5000 条可见 type1 → 5000 message upsert + 5000 channel 未读 +1
                // GuardedBump（离线未读落 DB，非仅 emit）= 10000 ops。
                use helix_core::effect::StorageOp;
                let msg_upserts = ops
                    .iter()
                    .filter(|op| matches!(op, StorageOp::BatchUpsert(s) if s.table == "message"))
                    .count();
                let unread_bumps = ops
                    .iter()
                    .filter(|op| matches!(op, StorageOp::GuardedBump(s) if s.table == "channel"))
                    .count();
                assert_eq!(
                    msg_upserts, GAP as usize,
                    "5000 条可见 type1 须 5000 message upsert"
                );
                assert_eq!(
                    unread_bumps, GAP as usize,
                    "A3：5000 条可见 type1 须各 1 条 channel 未读 +1 GuardedBump（CAP-9 落 DB）"
                );
                assert_eq!(
                    ops.len(),
                    (GAP as usize) * 2,
                    "persist batch = 5000 msg + 5000 bump"
                );
                Some(*corr)
            }
            _ => None,
        })
        .expect("sync events must yield one Persist batch");
    assert_eq!(
        module.cursor_for(channel_id(1)),
        Some(0),
        "cursor must NOT advance before PortReply Ok"
    );

    // 4. Persist 成功回报 → commit cursor 到 5000 + flush buffer 中的 5001
    let commit_tick = Tick::PortReply {
        corr: persist_corr,
        outcome: PortOutcome::Ok(ReplyBytes::default()),
    };
    sink.clear();
    let start_commit = Instant::now();
    module
        .handle(&commit_tick, 1_001_200, &mut sink)
        .expect("persist ok");
    let commit_elapsed = start_commit.elapsed();

    assert_eq!(
        module.cursor_for(channel_id(1)),
        Some(GAP + 1),
        "cursor must reach 5001 after commit + buffer flush"
    );

    eprintln!("\n[stress_gap_5000_sync_backfill]");
    eprintln!(
        "  sync response    : {} events, {} bytes JSON",
        GAP, resp_len
    );
    eprintln!(
        "  parse+ops step   : {:.3} ms ({:.0} events/s)",
        parse_elapsed.as_secs_f64() * 1e3,
        GAP as f64 / parse_elapsed.as_secs_f64()
    );
    eprintln!(
        "  commit+flush step: {:.3} ms",
        commit_elapsed.as_secs_f64() * 1e3
    );
    eprintln!(
        "  end-to-end (module side): {:.3} ms",
        (parse_elapsed + commit_elapsed).as_secs_f64() * 1e3
    );
}
