# 全 Gate 公共验收合同

每个 Gate 的首次纯 V2与删除后二次运行都必须满足：

- action、Tick、Effect、HTTP/WS、Persist、Frame、C++ apply、QuickJS UI 可按同一 correlation 串联。
- `schemaVersion=2`，`v1Frames=0`，raw business fallback 为 0。
- producer 与 consumer 两端均不接受 V1；V1/unknown schema 注入 fail-closed。
- PersistErr、epoch/revision gap、duplicate transaction、late receipt 保留最后一致 VM。
- QuickJS UI 只读 Surface VM；不得查询 row 表或从 raw event 重算业务状态。
- actor/observer、PG/SQLite 与移动 UI 证据来自同一 candidate 和 run window。

正式 Spec 的公共完成判据还必须包含：

- 19 个业务 UC、25 个机器 Gate 全部进入 profile；`G03/G03a` 明确不在分母。
- Helix/Mobile/source lock/headers/Android/iOS/Matrix/command contract/fixtures 为同一 source pin。
- `v1Frames`、`v2Frames`、`rawBusinessFallbacks`、frame reject/resync/ack 有机器计数，不靠日志目测。
- 实际 Venus 页面路径与 renderer key 已登记；demo/test binding 不替代产品 UI。
- 删除后二次运行使用新 candidate、run id、artifact root 与 evidence epoch，不能复用首次产物。
