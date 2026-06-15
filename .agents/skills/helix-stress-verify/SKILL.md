---
name: helix-stress-verify
description: 跑 helix 承压验收并解读数字。当改了热路径（engine_loop / storage / gate 状态机）、修性能问题、或需要回答"能不能扛并发"时使用。
---

# 承压验收跑法

## 三件套 + bench

```bash
# 内核步进/乱序/sync（helix-im，纯状态机）
cargo test --release -p helix-im --test stress_test -- --nocapture --test-threads=1
# 存储写吞吐 + 写放大（driver-native）
cargo test --release -p helix-driver-native --test stress_storage -- --nocapture --test-threads=1
# 端到端泵：裸灌死锁回归 + graceful drain（driver-native）
cargo test --release -p helix-driver-native --test stress_engine -- --nocapture --test-threads=1
# criterion 历史对比
cargo bench -p helix-im -p helix-driver-native
```

## 基线（2026-06-11，E1-E4 修复后；机器 i7-13700K，release=opt-level"z"）

| 指标 | 基线 | 跌破即回归 |
|---|---|---|
| 内核 step in-order | ~53 万帧/秒 | < 30 万 |
| 乱序 buffer/flush | ~95 万事件/秒 | < 50 万 |
| batch_upsert 5000 行 | 40-60ms（写放大 ~1.0x） | > 150ms |
| monotonic_upsert | ~7.4K/s（E5 修后应 ≥30K） | — |
| 泵 10k 裸灌 | 全闭环不死锁 | 任何冻结 |
| graceful drain | 关闭后查库全落盘 | 任何缺行 |

## 规则

1. **性能主张必须带数字**（修复前/后对照），数字进 commit message 验证节
2. 改 engine_loop/storage 必跑 stress_engine + stress_storage
3. 完整背景与瓶颈清单：`docs/过会方案/helix-并发承压评估-2026-06-11.md`
