# 全 Gate 公共验收合同

每个 Gate 的首次纯 V2与删除后二次运行都必须满足：

- action、Tick、Effect、HTTP/WS、Persist、Frame、C++ apply、QuickJS UI 可按同一 correlation 串联。
- `schemaVersion=2`，`v1Frames=0`，raw business fallback 为 0。
- producer 与 consumer 两端均不接受 V1；V1/unknown schema 注入 fail-closed。
- PersistErr、epoch/revision gap、duplicate transaction、late receipt 保留最后一致 VM。
- QuickJS UI 只读 Surface VM；不得查询 row 表或从 raw event 重算业务状态。
- actor/observer、PG/SQLite 与移动 UI 证据来自同一 candidate 和 run window。
