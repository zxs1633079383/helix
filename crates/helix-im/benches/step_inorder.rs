//! criterion 基准：内核步进吞吐（Bun 对齐 3.4——性能主张必须带数字）
//!
//! 与 tests/stress_test.rs 的 stress_inorder 同构，criterion 形态便于
//! 历史对比与回归检测。跑法：`cargo bench -p helix-im`
// 基准测量代码：clippy.toml deny 的显式豁免（与压测文件同理）
#![allow(clippy::disallowed_methods, clippy::disallowed_macros)]

use criterion::{criterion_group, criterion_main, Criterion, Throughput};
use helix_core::tick::InboundBytes;
use helix_core::{EffectSink, Module, Tick};
use helix_im::module::ImConfig;
use helix_im::state::ChannelId;
use helix_im::ImModule;

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
        serde_json::to_vec(&payload).expect("build json").into(),
    ))
}

fn bench_step_inorder(c: &mut Criterion) {
    const N: u64 = 10_000;
    let ticks: Vec<Tick> = (1..=N).map(|s| make_inbound_tick(42, s)).collect();

    let mut group = c.benchmark_group("im_step");
    group.throughput(Throughput::Elements(N));
    group.bench_function("inorder_10k", |b| {
        b.iter(|| {
            let mut module = make_module(42);
            let mut fx = EffectSink::new();
            for (i, t) in ticks.iter().enumerate() {
                fx.clear();
                module
                    .handle(t, 1_000_000 + i as u64, &mut fx)
                    .expect("handle");
            }
        });
    });
    group.finish();
}

criterion_group!(benches, bench_step_inorder);
criterion_main!(benches);
