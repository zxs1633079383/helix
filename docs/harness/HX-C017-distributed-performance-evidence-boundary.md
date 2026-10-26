---
id: HX-C017
title: 性能证据必须保持逐跳 correlation、确定性顺序与负载窗口边界
status: active
severity: major
scope: helix
created: 2026-07-13
source: 2026-07-13 三仓全链路性能复盘拍板
recurrence_count: 1
trigger_paths:
  - "crates/helix-im/src/**"
  - "crates/helix-driver-*/src/**"
  - "crates/helix-im/tests/**"
trigger_keywords: ["performance", "latency", "corr_key", "load_duration", "hot key"]
related: [HX-C009, HX-C010, HX-C011]
graduates_to: [AGENTS.md]
---

# HX-C017 — 分布式性能证据边界

## §1 触发场景

压测、延迟优化、批处理、并发 effect、发送/同步/投影链路性能分析。

## §2 背景

2026-07-13 服务端 5 RPS 诊断表明：最终投影/WS 延迟不能说明卡点，负载时长与 drain 混写会夸大结论；多 channel 并行也不能证明单 channel 保序容量。

## §3 Required / Forbidden

- 必须保留可跨 HTTP/WS/Effect/Projection 的 correlation key，性能优化不得破坏确定性顺序。
- 必须把客户端命令发出、端口等待、core step、effect 执行、投影 emit 分段记录。
- 热点单领域键与多领域键并行分别回放。
- 禁止在 core 读取墙钟或引入运行时指标依赖；时间与探针仍由 host/driver 注入。
- 禁止用一次最终 P95 反推 Helix core 是瓶颈。

## §4 Verification

```bash
cargo test -p helix-im
rg -n 'Instant::now|SystemTime::now' crates/helix-core crates/helix-im
```

回放证据必须包含 `load_duration/drain_duration`，相同输入仍产生 byte-identical、有序 Effect/Projection。

## §5 Recurrence Log

| 日期 | session/commit | 现象 | 根因 |
|---|---|---|---|
| 2026-07-13 | 性能复盘 | 只看到最终 WS 尾延迟 | 服务端逐跳指标和测试窗口边界未统一 |

## §6 关联

- 全局 `/distributed-performance-evidence`
- cses-im-server C018；loopforge C016

## §7 历史与演进

- 2026-07-13 用户拍板后 active。
