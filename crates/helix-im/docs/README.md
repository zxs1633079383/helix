# helix-im docs（IM 模块专属文档）

> 本目录只放 **helix-im 业务模块**自己的设计文档（IM 范畴）。
> core 层通用契约在仓库根 `docs/`（如 [`docs/过会方案/helix-eventbus-下游能力契约.md`](../../../docs/过会方案/helix-eventbus-下游能力契约.md)）；本目录是它在 IM 侧的**落地实例**。

| 目录/文档 | 一句话 |
|---|---|
| [`sync-v2-能力与缺口状态.md`](./sync-v2-能力与缺口状态.md) | **落地真相快照**：HTTP/WS/Storage 能力 + sync-v2 各 Phase 进度 + 8 缺口 + 明天代办（优先级排序）|
| [`eventbus/`](./eventbus/) | IM EventBus 设计：场景全集（结合现网 cses-client）+ 下游消费者域卡 |

## 维护铁律

- **域卡 = 消费侧契约**：描述"下游域拿 IM 事件能干什么、需要哪些上下文"，不写 UI 实现细节。
- **事件 type 唯一源**：所有 `im:*` 事件名以 `src/acl/to_effect.rs:140` 的生成点为准；域卡引用必须与之一致，漂移即修。
- **新增下游域** → 照域卡模板（8 字段）补一张，强制过 §0 反推判据。
