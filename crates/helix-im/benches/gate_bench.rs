//! criterion 基准：乱序增量同步 + 发送对账两条热路径（头号风险，必须有可签数字）
//!
//! 现有 step_inorder.rs 只 bench 了顺序步进（连续 seq 直接放行的乐观快路径）。
//! 纪要头号风险 = **乱序增量同步**（gate 缓冲 → 1s gate → sync backfill → 批量落库 →
//! commit + flush）——最该有可签数字却没 criterion。本文件补两条：
//!
//! - `bench_gate_backfill`：乱序场景 B（与 tests/stress_test.rs::stress_gap_5000_sync_backfill
//!   同构）。喂一条远端 seq=N+1 → buffer + arm gate；gate timer 超时 → Http{sync/notify}；
//!   PortReply 灌入 N events 的 sync 响应（parse + N 个 BatchUpsert op 构造）；
//!   再 PortReply Ok 触发 commit cursor + 从 buffer flush。度量整条 flush 链吞吐
//!   （events/s）—— 乱序缓冲在一次同步里被 backfill 落库的速率。
//! - `bench_send_echo_reconcile`：发送对账（与 tests/send_test.rs 同构，公开 API 可达路径）。
//!   im_send_message 命令（乐观落库 P1 + Http H1 + 15s timer + Emit）→ PortReply Ok(P1)
//!   把 temporaryId 对应的 PendingSend 推进到 Sending。度量「发送命令 + P1 对账回报」
//!   一来一回的吞吐（reconciles/s）。
//!
//! ## 方法学铁律（criterion 测量纯净性）
//!
//! - owned 状态（ImModule / 预构建 Tick / sync 响应 bytes）一律在 `iter_batched` 的
//!   **setup 闭包**里构造，measured 闭包只含 `module.handle(...)` 调用链 —— JSON 构造、
//!   Vec 分配、模块新建都不计入测量。
//! - `Throughput::Elements(N)`：报告按「乱序事件数 / 对账次数」归一，便于跨规模对比。
//! - warmup ≥ 3s + 充足 sample（gate_backfill 单 iter 重，sample 调小避免超时）。
//! - 纯内核 sans-IO：注入的 driver 行为用预构造的 PortReply Tick 模拟，**不起真 I/O**。
//!
//! ## 回归判据（CI 可签）
//!
//! - `gate/backfill`：基线 median 见首次 `cargo bench` 输出（events/s）。
//!   **median 退化 > 2x（吞吐腰斩）即红**——意味着乱序 backfill 热路径引入了
//!   read-modify-write / 多余分配 / O(N²) flush。
//! - `send/echo_reconcile`：基线 median 见首次输出（reconciles/s）。
//!   **median 退化 > 2x 即红**——发送对账每条消息命中，劣化直接放大到群聊规模。
//!
//! 跑法：`cargo bench -p helix-im --bench gate_bench`

// 基准测量代码：clippy.toml deny 的显式豁免（与 step_inorder / stress 文件同理）
#![allow(clippy::disallowed_methods, clippy::disallowed_macros)]

use criterion::{criterion_group, criterion_main, BatchSize, Criterion, Throughput};

use helix_core::effect::Correlation;
use helix_core::tick::{AppCommand, InboundBytes, PortOutcome, ReplyBytes};
use helix_core::{Effect, EffectSink, Module, Tick};
use helix_im::module::ImConfig;
use helix_im::state::ChannelId;
use helix_im::ImModule;

// ─── 辅助构造（全部在 setup 阶段调用，不计入测量）──────────────────────────────

fn make_module(channel_id: u64) -> ImModule {
    let mut module = ImModule::new(ImConfig {
        ws_url: "ws://bench".to_string(),
        api_base_url: "http://bench".to_string(),
        ping_interval_ms: 8_000,
        send_timeout_ms: 15_000,
    });
    module.register_channel(ChannelId(channel_id), 0);
    module
}

/// 构造 posted WS 帧（与 tests/common.rs::inbound_event 同构，payload 约 150 字节）
fn make_inbound_tick(channel_id: u64, seq: u64) -> Tick {
    let payload = serde_json::json!({
        "event": "posted",
        "data": {
            "channel_id": channel_id,
            "seq": seq,
            "event_type": 1,
            "msg_id": format!("msg_{}", seq),
            "actor_id": "user_1",
            "created_at": 1000000_u64,
            "payload": null
        }
    });
    Tick::Inbound(InboundBytes(
        serde_json::to_vec(&payload)
            .expect("build inbound json")
            .into(),
    ))
}

/// 构造 im_send_message 命令 Tick（与 tests/common.rs::send_message_cmd 同构）
fn send_message_cmd(channel_id: u64, temporary_id: &str, text: &str) -> Tick {
    let payload = serde_json::json!({
        "channel_id": channel_id,
        "temporary_id": temporary_id,
        "text": text
    });
    Tick::Command(AppCommand::new(
        "im_send_message",
        serde_json::to_vec(&payload).expect("build cmd json"),
    ))
}

/// 构造含 GAP events 的 sync 响应 bytes（与 stress_gap_5000_sync_backfill 同构）。
/// 在 setup 阶段预构建，measured 闭包只 parse。
fn build_sync_response(channel_id: u64, gap: u64) -> bytes::Bytes {
    let events: Vec<serde_json::Value> = (1..=gap)
        .map(|s| {
            serde_json::json!({
                "channel_id": channel_id,
                "seq": s,
                "event_type": 1,
                "msg_id": format!("msg_{}", s),
                "actor_id": "user_1",
                "created_at": 1000000_u64,
                "payload": null
            })
        })
        .collect();
    // 真 Go wire：{status:SUCCESS, data:{entries:[SyncEntry], serverTime}}。
    // channelId 用 from_test_u64 同款 26 字符零填充形态，匹配注册的 ChannelId。
    let resp = serde_json::json!({
        "status": "SUCCESS",
        "data": {
            "entries": [{
                "channelId": format!("{:026}", channel_id),
                "kind": "events",
                "events": events,
                "nextSeq": gap,
                "resetTo": serde_json::Value::Null,
            }],
            "serverTime": 1_700_000_000_i64
        }
    });
    serde_json::to_vec(&resp)
        .expect("build sync response")
        .into()
}

/// 在 module 上把 gate 推进到「等待 sync 响应」状态，返回 sync 的 Http corr。
/// 用于 setup：喂远端 seq=gap+1（buffer + arm gate）→ gate timer 超时（吐 Http）。
/// 这部分准备工作不计入 backfill measured 区间（backfill 度量的是
/// PortReply 灌入 sync 响应 → 批量落库 → commit + flush 这条核心链）。
fn arm_gate_and_get_sync_corr(module: &mut ImModule, channel_id: u64, gap: u64) -> Correlation {
    let mut sink = EffectSink::new();

    // 1) 喂远端 seq=gap+1 → buffer + arm gate timer
    let far_tick = make_inbound_tick(channel_id, gap + 1);
    sink.clear();
    module
        .handle(&far_tick, 1_000_000, &mut sink)
        .expect("handle far event");
    let gate_timer = sink
        .as_slice()
        .iter()
        .find_map(|e| match e {
            Effect::ScheduleTimer { id, .. } => Some(*id),
            _ => None,
        })
        .expect("gate timer must be armed");

    // 2) gate timer 超时 → Http{sync/notify}
    sink.clear();
    module
        .handle(&Tick::Timer(gate_timer), 1_001_000, &mut sink)
        .expect("gate timeout");
    sink.as_slice()
        .iter()
        .find_map(|e| match e {
            Effect::Http { corr, .. } => Some(*corr),
            _ => None,
        })
        .expect("sync/notify Http must be emitted")
}

// ─── 基准 1：乱序增量同步 backfill（sync 响应 → 批量落库 → commit + flush）─────────

fn bench_gate_backfill(c: &mut Criterion) {
    const GAP: u64 = 5_000;

    let mut group = c.benchmark_group("gate");
    group.throughput(Throughput::Elements(GAP));
    group.warm_up_time(std::time::Duration::from_secs(3));
    group.sample_size(30); // 单 iter 重（5000 op 落库 + flush），保守 sample 防超时

    group.bench_function("backfill_5000", |b| {
        b.iter_batched(
            // ── setup（不计入测量）：构造 module + 推进 gate 到等 sync + 预建响应 bytes ──
            || {
                let mut module = make_module(1);
                let sync_corr = arm_gate_and_get_sync_corr(&mut module, 1, GAP);
                let resp_bytes = build_sync_response(1, GAP);
                let sink = EffectSink::new();
                (module, sync_corr, resp_bytes, sink)
            },
            // ── measured：PortReply 灌入 sync 响应 → 批量落库 → commit + flush ──
            |(mut module, sync_corr, resp_bytes, mut sink)| {
                // a) sync 响应 PortReply：parse + GAP 个 BatchUpsert op 构造 → 吐 Persist
                let reply_tick = Tick::PortReply {
                    corr: sync_corr,
                    outcome: PortOutcome::Ok(ReplyBytes(resp_bytes)),
                };
                sink.clear();
                module
                    .handle(&reply_tick, 1_001_100, &mut sink)
                    .expect("sync reply");
                let persist_corr = sink
                    .as_slice()
                    .iter()
                    .find_map(|e| match e {
                        Effect::Persist { corr, .. } => Some(*corr),
                        _ => None,
                    })
                    .expect("sync events must yield one Persist batch");

                // b) Persist 成功回报 → commit cursor 到 GAP + flush buffer 中的 GAP+1
                let commit_tick = Tick::PortReply {
                    corr: persist_corr,
                    outcome: PortOutcome::Ok(ReplyBytes::default()),
                };
                sink.clear();
                module
                    .handle(&commit_tick, 1_001_200, &mut sink)
                    .expect("persist ok");

                // 防优化掉：黑盒消费 cursor（不进入测量统计语义，仅阻止 DCE）
                criterion::black_box(module.cursor_for(ChannelId(1)));
            },
            BatchSize::SmallInput,
        );
    });
    group.finish();
}

// ─── 基准 2：发送对账（im_send_message 乐观 + P1 PortReply 推进）──────────────────

fn bench_send_echo_reconcile(c: &mut Criterion) {
    // 每个 measured iter 跑 N 条「发送命令 + P1 对账回报」一来一回，
    // Throughput 按 N 归一报告 reconciles/s。
    const N: u64 = 1_000;

    // 预构建 N 条发送命令（temporaryId 各异），setup 阶段一次性建好。
    let cmds: Vec<Tick> = (0..N)
        .map(|i| send_message_cmd(1, &format!("tmp_{}", i), "hello"))
        .collect();

    let mut group = c.benchmark_group("send");
    group.throughput(Throughput::Elements(N));
    group.warm_up_time(std::time::Duration::from_secs(3));

    group.bench_function("echo_reconcile_1000", |b| {
        b.iter_batched(
            // ── setup：全新 module + 复位 sink（owned 状态构造不计入测量）──
            || (make_module(1), EffectSink::new()),
            // ── measured：N × (发送命令 → 取 P1 corr → P1 PortReply Ok 推进) ──
            |(mut module, mut sink)| {
                for cmd in &cmds {
                    // a) 发送命令：吐 Persist(P1) + Http(H1) + ScheduleTimer(T1) + Emit
                    sink.clear();
                    module.handle(cmd, 1_000_000, &mut sink).expect("send cmd");
                    let p1_corr = sink
                        .as_slice()
                        .iter()
                        .find_map(|e| match e {
                            Effect::Persist { corr, .. } => Some(*corr),
                            _ => None,
                        })
                        .expect("send must yield P1 Persist");

                    // b) P1 落库成功对账：PendingSend 推进到 Sending（temporaryId 对账点）
                    let p1_ok = Tick::PortReply {
                        corr: Correlation::from_raw(p1_corr.raw()),
                        outcome: PortOutcome::Ok(ReplyBytes::default()),
                    };
                    sink.clear();
                    module.handle(&p1_ok, 1_000_100, &mut sink).expect("p1 ok");
                }
                criterion::black_box(module.pending_send_count());
            },
            BatchSize::SmallInput,
        );
    });
    group.finish();
}

criterion_group!(benches, bench_gate_backfill, bench_send_echo_reconcile);
criterion_main!(benches);
