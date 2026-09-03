---
name: helix-new-module
description: 在 helix 上新增第二/第三业务模块（如 meeting/presence/audit）的完整 pattern。当用户要求"加一个业务模块 / 接入新业务到 helix core"时使用。
---

# 新增业务模块 pattern

## 步骤（core 零改动是验收标准）

1. **新 crate**：`crates/helix-<name>/`，Cargo.toml 只依赖 helix-core + serde/bytes/tracing（抄 helix-im 的，含 `[lints.clippy]` deny 块）
2. **实现 Module trait** 5 方法：`name` / `accepts`（排他认领自己的帧与命令，与现有模块零重叠）/ `handle`（同步纯函数，只 `out.push(Effect::…)`）/ `on_start`（握手/读水位/arm 心跳）/ `on_stop`（**必须 CancelTimer 自己 arm 的全部 timer**）
3. **ID 分配**：模块内单调 `next_corr`/`next_timer`，起点偏移避开其他模块（BLK-1b 约定：im 从 1/100 起，新模块隔 10000）
4. **异步全走 corr**：吐 `Effect::{Persist,Http}{corr}` → `handle_port_reply` 按自维护的 corr→意图路由表分发；PersistFire 仅限幂等写
5. **挂载**：宿主 `shell.register(XxxModule::new(config))` 一行
6. **测试**：抄 helix-im `tests/common.rs` 剧本工具——喂 Tick 序列断言 Effect 序列；乱序场景加 proptest

## 参考实现

- 完整范例：`crates/helix-im/src/module.rs`（路由表 pattern）+ 架构设计 §6.4 会议模块全链路图
- 验收：对应 crate 的聚焦测试、格式检查和 `git diff --check` 通过 + core 的 `git diff` 为空（证明零改内核）；`scripts/gate.sh` 仅在用户或活动 CI/发布合同明确要求时追加

## 红线

- handle 内禁 await/IO/spawn/ambient 时间（clippy 会拦）
- 不 import 其他业务模块 crate（模块间经 core 事件总线协作）
- 对 PortReply 到达顺序零假设（幂等设计）
