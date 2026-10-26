# G01 远端 PG 与本地 SQLite

## 当前 persistence 合同

| 合同名 | 物理边界 |
|---|---|
| `message` | 领域 persistence 名称；物理表须以 StorageOp/schema 当前代码核验 |
| `pending_send` | Helix 内存 PendingSend；SQLite 状态位于 message 行 |

## 提交规则

- Strict Matrix 的 persistence 是领域合同名，不自动等于 SQLite 或 PG 表名。
- 本地写入必须由 matching `PersistOk` 关闭 barrier；远端写入还需权威响应或后续 WS/sync 事实。
- cursor 不得在消息或投影尚未持久化时单独前移。
- 静态文档不证明真实 PG DSN、SQLite 文件或当前进程已写入；运行验收必须单独取证。
