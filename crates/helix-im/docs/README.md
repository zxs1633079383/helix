# helix-im docs（IM 模块专属文档）

> 本目录只放 **helix-im 业务模块**自己的设计文档（IM 范畴）。
> core 层通用契约在仓库根 `docs/`（如 [`docs/过会方案/helix-eventbus-设计.md`](../../../docs/过会方案/helix-eventbus-设计.md)）；本目录是它在 IM 侧的**落地实例**。
> 2026-07-13 熵减规则：本页是 IM 模块 docs 的唯一入口。已删除的 takeover/sync-v2 阶段稿不再作路由；当前 UC 事实、投影字段和发布证据分别进入下方三个稳定入口。

## 当前事实入口

| 目录/文档 | 一句话 |
|---|---|
| [`前端UC投影指令与数据清单-最新代码分析-2026-07-09.md`](./前端UC投影指令与数据清单-最新代码分析-2026-07-09.md) | **UC 消费视角真源**：Gate、command、Effect、HTTP、WS、Go DB、Helix storage 与 UI VM 的当前综合口径 |
| [`Strict Matrix/README.md`](./Strict%20Matrix/README.md) | **UC 发布真源入口**：机器合同、生成矩阵、远端存储真源和持久证据包；缺任一证据面即 Red |
| [`projection-schema.md`](./projection-schema.md) | **投影 payload 契约真源（C2 冻结）**：21 个 `im:__bus__` 投影 emit 工厂字段集 + 信号/透传/控制三类 + 命名陷阱词典。配 `tests/projection_schema_test.rs` 强制对账（缺失/多余即 fail）|
| [`uc-automation/uc-command-set-spec.md`](./uc-automation/uc-command-set-spec.md) | 当前 UC command/query/event 契约入口；新增/删改 UC 先对齐这里，再改测试和下游文档 |

## 支撑材料与历史材料

| 目录/文档 | 一句话 |
|---|---|
| [`csesapi-migration/`](./csesapi-migration/真机curl真源.md) | 迁移真源：真机 curl 6 抓 + createPosts 三方证据定论（附录 A）+ [全量图谱](./csesapi-migration/full-map/00-README-index.md) + [缺口矩阵](./csesapi-migration/full-map/30-capability-gap-matrix.md) |
| [`design/`](./design/README.md) | mermaid 设计图：WS 入站管线 / send echo 对账 / sync 流 / gate 状态机 |
| [`eventbus/`](./eventbus/) | IM EventBus 设计：场景全集（结合现网 cses-client）+ 下游消费者域卡 |
| [`uc-automation/uc-test-order-spec.md`](./uc-automation/uc-test-order-spec.md) | 历史 DAG / 执行顺序参照；当前 UC 完整性以 `uc-command-set-spec.md` 为准 |

## 维护铁律

- **域卡 = 消费侧契约**：描述"下游域拿 IM 事件能干什么、需要哪些上下文"，不写 UI 实现细节。
- **事件 type 唯一源**：所有 `im:*` 事件名以 `src/acl/to_effect.rs:140` 的生成点为准；域卡引用必须与之一致，漂移即修。
- **新增下游域** → 照域卡模板（8 字段）补一张，强制过 §0 反推判据。
- **当前 green 不来自本文**：本文只路由文档；runtime green 必须来自测试/gate/reviewer/collector。
