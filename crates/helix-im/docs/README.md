# helix-im docs（IM 模块专属文档）

> 本目录只放 **helix-im 业务模块**自己的设计文档（IM 范畴）。
> core 层通用契约在仓库根 `docs/`（如 [`docs/过会方案/helix-eventbus-设计.md`](../../../docs/过会方案/helix-eventbus-设计.md)）；本目录是它在 IM 侧的**落地实例**。

| 目录/文档 | 一句话 |
|---|---|
| [`takeover-plan/`](./takeover-plan/00-README.md) | **全量 HTTP+WS 接管剧本（P0-P7）**：架构状态机 + 分阶段路线 + 测试策略 + cses-client 接入方案 + 阶段表。接管终态 = 89 命令 + 19 WS + Tauri 四接缝 + cses CI-1~CI-3 已接入（CI-4 待 Go）|
| [`projection-schema.md`](./projection-schema.md) | **投影 payload 契约真源（C2 冻结）**：21 个 `im:__bus__` 投影 emit 工厂字段集 + 信号/透传/控制三类 + 命名陷阱词典。配 `tests/projection_schema_test.rs` 强制对账（缺失/多余即 fail）|
| [`sync-v2-能力与缺口状态.md`](./sync-v2-能力与缺口状态.md) | **落地真相快照**：HTTP/WS/Storage 能力 + sync-v2 各 Phase 进度 + 缺口 + 接管收官（§4）|
| [`csesapi-migration/`](./csesapi-migration/真机curl真源.md) | 迁移真源：真机 curl 6 抓 + createPosts 三方证据定论（附录 A）+ [全量图谱](./csesapi-migration/full-map/00-README-index.md) + [缺口矩阵](./csesapi-migration/full-map/30-capability-gap-matrix.md) |
| [`design/`](./design/README.md) | mermaid 设计图：WS 入站管线 / send echo 对账 / sync 流 / gate 状态机 |
| [`eventbus/`](./eventbus/) | IM EventBus 设计：场景全集（结合现网 cses-client）+ 下游消费者域卡 |

## 维护铁律

- **域卡 = 消费侧契约**：描述"下游域拿 IM 事件能干什么、需要哪些上下文"，不写 UI 实现细节。
- **事件 type 唯一源**：所有 `im:*` 事件名以 `src/acl/to_effect.rs:140` 的生成点为准；域卡引用必须与之一致，漂移即修。
- **新增下游域** → 照域卡模板（8 字段）补一张，强制过 §0 反推判据。
