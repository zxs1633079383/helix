# InteractionFrame 文档路由

本文件只为下列两个目录增加专项约束，不改变 `docs/` 其他文档的既有真源和写法：

- `interaction-frame-v2-全量替换-按UC/`
- `interaction-frame-v2-移动端全量替换-按UC/`

## 所有权边界

| 目录 | 平台所有权 | 必须展开 | 不得反向定义 |
|---|---|---|---|
| `interaction-frame-v2-全量替换-按UC/` | PC Angular/Tauri | Angular component、DOM selector、Native SurfaceRuntime、PC UI 效果与证据 | Mobile QuickJS/C++ 页面和 Flutter Widget |
| `interaction-frame-v2-移动端全量替换-按UC/` | Mobile Venus/QuickJS/C++ | Venus 页面生命周期、QuickJS API、`MobileImSdk/CoreBridge/SurfaceRuntime`、FFI batch、移动 UI 效果与证据 | Angular DOM、Helix 领域决策、Go wire 或数据库语义 |

## 共享真源

1. Helix `ActionBinding -> ActionInvocationEnvelope -> Tick -> Effect -> PersistAtomic/PersistOk -> SurfaceComposer -> InteractionFrameV2` 由 PC 目录对应 Gate 和当前 Helix 源码共同定义。
2. Go HTTP、WS、远端 PostgreSQL 与 Helix SQLite 字段以当前 Go/Helix 源码为准。Mobile 文档可以索引复用，不能复制后自行改义。
3. Mobile 只负责平台适配：`InteractionFrameV2` 进入 C ABI batch 后，由 C++ 校验、原子应用并向 QuickJS 暴露只读 Surface VM；业务 reducer 不回到 C++/JS。
4. Flutter 仅作为 Mobile 公共架构的外联消费面。逐 Gate UI 真源是 Venus QuickJS 页面，不凭空创建 Widget 树。

## V1 清退顺序

两个目录都遵守同一顺序：

1. 冻结逐 Gate V1 类图、流程、数据和生产旁路。
2. 同时切断 V1 producer 与 V1 consumer 的生产可达路径。
3. 在固定 candidate 上完成首次纯 V2 验收，要求 `v1Frames=0`、raw business fallback 为 0。
4. 审批 deletion inventory 后物理删除 V1 writer、reader、normalize、fixture 与兼容测试。
5. 使用新 candidate 和新 evidence epoch 完成删除后二次验收。

`schemaVersion` 字段不得删除；V2 必须携带 `schemaVersion=2`。只删除
`InteractionFrame schemaVersion=1` 的生产、消费和兼容支持。不得误删
`InteractionProtocol/v1`、`ActionInvocationEnvelope/v1`、Strict Matrix 文件格式版本或其他独立版本。

## 写作与证据

- 25 个机器 Gate 必须独立、source-grounded，禁止复制标题后用通用内容冒充完成。
- Mobile 每个 Gate 必须有一张从 Venus/QuickJS 输入，经 Helix/Go/存储，再回到 C++/QuickJS UI 的连续总图；共享 Helix 内部细节可以链接 PC Gate，但总链不可割裂。
- 每个事实给出绝对源码路径和行号或稳定合同链接。无法证明时只允许
  `SOURCE_MISSING`、`CONTRACT_DRIFT`、`TARGET_DECISION_REQUIRED`、`NOT_APPLICABLE`。
- 共享字段发生冲突时 fail-closed，不得静默选择一端，也不得用 Mobile 当前 V1 reducer 反向定义 V2。
- 修改共享协议、Gate 范围或版本时，必须同时检查两个目录的范围、Gate 索引、下线清单和验收合同。
