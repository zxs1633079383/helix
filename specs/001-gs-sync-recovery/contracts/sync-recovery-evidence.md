# 合同：GS01-GS04 同步恢复证据

## 只读边界

所有状态检查只能通过 `mcp__helix_local_sqlite_ro__*` 和 `mcp__cses_pre_postgres_ro__*` 完成。查询必须限定租户、频道、序号范围和返回行数。禁止通过数据库写入、DDL、修改游标、插入账本、修改缓存或重启应用来制造预期结果。

## 证据记录

每次候选版本验证都必须在业务数据之外生成一份不可变证据记录：

| 字段 | 必需证据 |
|---|---|
| 身份 | 测试租户/操作者的匿名标识、会话 epoch，不得包含密钥或 token。 |
| 范围 | 频道 ID、`local_before`、`remote_head`、预期 `from_seq/to_seq`。 |
| 调用跳点 | 使用同一个 correlation 串联连接/登录、比较、HTTP/WS 输入、`PersistAtomic` 回执、组装、Frame 应用和 ack。 |
| 本地事实 | 从 `channel`、`channel_member`、`message`、`channel_event_ledger`、`channel_sync_coverage`、`channel_event_cursor` 查询的有界结果。 |
| 远端事实 | 从 PG 的 `channel`、`channel_member`、`post`、`channel_event`、`channel_event_head` 查询的有界结果。 |
| UI | PC DOM 对全局同步状态和恢复后 Surface VM 的断言；移动端 Frame fixture 应用结果。 |
| 测试 | 命令、开始/结束时间、墙钟耗时、通过/失败/跳过数量和首个失败。 |
| 结论 | `PASS | FAIL | BLOCKED`、首个偏差和下一步行动。 |

## 必须完成的对账

| 对账面 | 通过条件 |
|---|---|
| 权威头 | 恢复成功后，本地已提交游标必须等于远端权威头。 |
| 事件范围 | 在选定 coverage 内，账本对每个远端 `(channel_id,event_seq)` 恰好保留一个规范 hash；数量和范围必须与 coverage 一致。 |
| 业务事实 | 选定的消息、频道、成员行必须与远端规范值或已记录的投影映射一致。 |
| 原子失败 | 注入持久化错误后，事实、账本、coverage、游标和最终 Frame 都必须停留在批次执行前。 |
| UI 状态 | 工作期间同步状态是全局的；只有提交成功后才出现恢复完成状态；陌生频道在 hydration 提交前必须没有或禁用发送动作。 |
| 身份隔离 | A 切换登录到 B 后，B 不能看到 A 的 Frame 槽位、账本范围或 ActionBinding。 |

## 失败分类

| 条件 | 结论 | 自动处理 |
|---|---|---|
| 本地落后远端且恢复未完成 | `FAIL` | 保留证据；合同允许时重试权威恢复。 |
| 本地领先远端或连续范围非法 | `BLOCKED` | 停止自动对账并请求人工决策。 |
| 缺少数据库只读权限或查询限制能力 | `BLOCKED` | 报告能力缺口，禁止降级成“测试已通过”。 |
| Frame schema/revision/epoch 非法 | `FAIL` | 请求指定范围的 snapshot，禁止消费 raw emit。 |
| 持久化失败 | 只有游标和 Frame 均未推进时，预期失败用例才算通过 | 记录首个失败写入或跳点。 |
| 需要数据库写入、触碰真实用户数据或改变已签字语义 | `BLOCKED` | 必须人工介入。 |

## 进度记录合同

连续实施或验证期间，至少每 30 分钟写一行简短状态：

```text
<时间戳> | 已完成: <完成的证据/工作> | 当前: <结果或阻塞> | 下一步: <一个具体操作>
```

这不是定时器、自动完成信号，也不能替代正式验收。

## 有界查询模板（T002）

所有占位符都由隔离证据运行提供，禁止替换成无界通配符。`:tenant`、`:tenant_actor_scope`、`:actor`、`:channel`、`:from_seq`、`:to_seq` 和 `:limit` 都是审计输入，且必须满足 `1 <= :limit <= 500`。`:tenant_actor_scope` 是由当前租户和操作者组成的长度前缀不透明恢复分区；它不是 SQL 表达式，必须作为一个完整值精确绑定。

```sql
-- 本地 SQLite：已提交游标、coverage、账本和业务事实。由只读 MCP 调用方绑定命名参数后分别执行。
SELECT channel_id, last_event_seq FROM channel_event_cursor
 WHERE channel_id = :channel LIMIT 1;
SELECT coverage_id, tenant_id, channel_id, from_seq, to_seq, event_count, facts_hash, correlation_id
  FROM channel_sync_coverage
 WHERE tenant_id = :tenant_actor_scope AND channel_id = :channel
   AND to_seq >= :from_seq AND from_seq <= :to_seq
 ORDER BY from_seq LIMIT :limit;
SELECT tenant_id, channel_id, event_seq, event_kind, message_id, event_hash, coverage_id
  FROM channel_event_ledger
 WHERE tenant_id = :tenant_actor_scope AND channel_id = :channel
   AND event_seq BETWEEN :from_seq AND :to_seq
 ORDER BY event_seq LIMIT :limit;
SELECT id, channel_id, user_id, type, create_at, update_at FROM message
 WHERE channel_id = :channel ORDER BY create_at, id LIMIT :limit;
SELECT channel_id, user_id, team_id, role, updated_at FROM channel_member
 WHERE channel_id = :channel AND team_id = :tenant ORDER BY user_id LIMIT :limit;
```

```sql
-- 远端 PostgreSQL 权威数据。准确的租户条件必须依据已验证的表结构选择；
-- 所有查询都必须包含频道和序号边界。
SELECT channel_id, event_seq FROM channel_event_head
 WHERE channel_id = :channel LIMIT 1;
SELECT channel_id, event_seq, event_type, msg_id, actor_id, create_at, update_at
  FROM channel_event
 WHERE channel_id = :channel AND event_seq BETWEEN :from_seq AND :to_seq
 ORDER BY event_seq LIMIT :limit;
SELECT id, channel_id, user_id, type, create_at, update_at FROM post
 WHERE channel_id = :channel ORDER BY create_at, id LIMIT :limit;
SELECT channel_id, user_id, team_id, role, update_at FROM channel_member
 WHERE channel_id = :channel AND team_id = :tenant ORDER BY user_id LIMIT :limit;
```

## 基线发现（不是 T050 候选版本证据）

在 `2026-07-21T21:36:00-07:00`，只读 MCP 检查了本地租户键 `444` 和 PG 元数据。SQLite 共报告 9 张表，其中包括 `channel`、`channel_member`、`message`，但不存在 `channel_event_cursor`、`channel_event_ledger` 或 `channel_sync_coverage`。PG 具备全部五张权威数据表；`channel_event` 暴露了 `channel_id`、`event_seq`、`event_type`、`msg_id`、`actor_id`、`payload`、`create_at` 和 `update_at` 字段。

这只能证明所需迁移和查询模板，**不能证明同步恢复真实运行过**。当前四平面结论为 `BLOCKED`：必须先让候选实现使用当前源码 pin，并在隔离测试数据库中具备全部必需本地表。

2026-07-21T21:36:00-07:00 | 已完成: T001 源码基线和 T002 有界只读证据合同 | 当前: 本地租户仍是 V2 前结构，四平面证据明确为 BLOCKED | 下一步: 在基础实现前冻结三仓 fixture 命名和 source-pin 合同

## T049 聚焦验证执行记录（2026-07-21）

所有时间戳均为 America/Los_Angeles 时区。下表中的 `PASS` 仅代表对应命令通过，不能替代只读四平面证据或真实 Go 链路。

| 验证面 | 命令 | 开始 / 结束 / 墙钟耗时 | 结果 |
|---|---|---|---|
| Helix 恢复 + V2 消费端 | `cargo test -p helix-im --test gs01_04_v2_sync_recovery_test --test interaction_frame_contract_test` | 23:01:52 / 23:02:09 / 17s | `PASS`：6 个恢复测试和 16 个 Frame 合同测试通过。 |
| Host 持久化合同 | `cargo test -p helix-driver-host --test im_event_persist` | 包含在同一次 23:01:52-23:02:09 执行中 / 17s | `PASS`：9 个测试通过。 |
| PC V2 DOM 与 fixture 对账 | `npx jest src/pages/message-v4/surface-runtime/__tests__/gs01-04-sync-recovery-dom.test.ts src/pages/message-v4/surface-runtime/__tests__/surface-runtime.service.test.ts src/pages/message-v4/surface-runtime/__tests__/surface-runtime.fixture-parity.test.ts src/pages/message-v4/surface-runtime/__tests__/projection-store.ingress.test.ts --runInBand` | 23:09:04 / 23:09:10 / 6s | `PASS`：4 个 suite、27 个测试通过。 |
| 移动端原生 V2 应用 | `make surface-runtime-gtest` | 23:06:46 / 23:07:11 / 25s | `PASS`，但有 1 个明确跳过项：38 个通过；由于未提供正式 Helix 源码对账输入，1 个 source-parity 测试跳过。 |
| 移动端 source-pin 脚本 | `node ./scripts/check-surface-runtime-source-pin.mjs` | 23:08:36 / 23:08:38 / 2s | `PASS` 仅覆盖已有 `b4339...` 基线 pin 和一个本地候选 V2 fixture hash；没有执行当前源码的显式对账。 |
| Flutter V2 入口 | `dart test test/surface_runtime_apply_test.dart` | 23:08:44 / 23:08:45 / 1s | `BLOCKED`：当前环境未安装 `dart`。 |
| PC Rust bridge 测试 | `cargo test -p cses-client ...` | 未完成 | `BLOCKED`：冷依赖构建已停止；源码测试已经存在，但本次没有可执行结果。 |
| 本地 SQLite + 远端 PG 四平面对账 | 本文中的有界只读查询 | 无法针对当前隔离 V2 候选版本执行 | `BLOCKED`：现有只读基线属于 V2 前结构，既没有当前候选 pin，也缺少本地 ledger/coverage 表。 |
| 真实 Go/WS correlation 链 | 连接/登录 -> 比较 -> PersistAtomic -> FrameApplied -> ack | 当前 worktree 未配置 | `BLOCKED`：没有可用且已授权的隔离测试身份和真实链路。 |

## T050 完成情况与切换决策

实现和任务交付已经完成到 T050。发布结论是 **`BLOCKED`**，不是 `PASS`/Green。因此，本 Feature 不允许物理删除 raw producer。

仍缺少的发布证据：

1. 将全部 GS01-GS04 V2 fixture 字节重新 pin 到当前 Helix 候选版本，并运行 PC/移动端正式 source-parity gate。当前移动端 pin 只是 V1 基线加本地候选 fixture hash，不是当前源码证明。
2. 准备完成迁移的隔离 SQLite 候选库和 PG 权威库只读权限，再使用一个匿名租户/操作者/频道范围，完成有界的游标、账本、coverage、业务事实和权威头对账。
3. 准备真实 Go/WS 测试链路及 Flutter/Dart 工具链，保留一条从登录一直贯穿到 `FrameApplied` 和 ack 的 correlation trace。

在上述证据补齐前，raw producer 的归属保持不变：channel gate 投影（`im:post:*`）、WebSocket post parser、`posts_update`、increment hydration、port-reply dispatch 和 capability projection。V2 消费端 gate 会拒绝 V1/raw fallback，但这不等于已经获得删除这些 producer 的授权。

2026-07-21T23:09:10-07:00 | 已完成: T020-T050 实现、Rust/PC/移动端聚焦验证和切换记录 | 当前: 切换被当前源码对账、只读四平面和真实 Go/WS 证据阻塞 | 下一步: 获取隔离候选 pin、只读查询范围和真实测试身份后重新执行本记录
