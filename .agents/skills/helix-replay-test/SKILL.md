---
name: helix-replay-test
description: 为 helix-im / helix 业务模块写回放（剧本）测试的标准写法。当需要测试业务状态机行为、复现 bug、或验证 Tick→Effect 序列时使用。
---

# 回放测试写法

核心思想：模块是 sans-IO 纯状态机——**喂 Tick 序列，断言 Effect 序列**，不起网络不起库，毫秒级。

## 模板

```rust
use helix_im::tests_common::*;   // tests/common.rs 的剧本工具

let mut m = make_module(channel_id);
let mut fx = EffectSink::new();

// 喂入站帧（serde_json 构造 posted/sync 帧，见 common.rs 辅助函数）
m.handle(&inbound_event(42, seq, EventKind::New), now, &mut fx).unwrap();
// 断言吐出的 Effect（用 matches! 按变体断言，不比对内部细节）
assert!(matches!(fx.as_slice()[0], Effect::PersistFire { .. }));

// 喂 I/O 回报（驱动协程续跑）
m.handle(&Tick::PortReply { corr, outcome: PortOutcome::Ok(reply) }, now, &mut fx).unwrap();
```

## 规则

1. 新业务逻辑**必须**配回放测试（修 bug = 先写复现剧本再修）
2. 乱序/重复/丢帧场景进 proptest（参考 `tests/proptest_gate.rs` 的不变量写法：cursor 单调 / 不丢消息 / 同输入同输出）
3. 回归测试文件头加锚注释：`// 回归锚: E{N}/B{N}（待办清单编号）`
4. scan/HTTP 回报的 bytes 构造必须对齐 driver 的 JSON 约定（`engine_loop.rs rows_to_reply_bytes`：`[{"col":val}]`；HTTP 信封 `{"status","headers","body"}`）
5. 时间用递增的假 now_ms，不用真实时间（确定性）

## 跑法

```bash
cargo test -p helix-im                       # 全量
cargo test -p helix-im --test proptest_gate  # 仅性质测试
```
