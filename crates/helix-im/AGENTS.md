# helix-im 局部指令（就近优先，全局见根 AGENTS.md）

> 📐 **历史设计图 → [`docs/design/`](docs/design/)**（旧模块布局与流程快照；当前行为先读源码、Spec 047 和当前运行时流程文档）。

## 本 crate 文档索引（`docs/` · 按需 Read）

- **⚪ V1/旧链历史参考** → [`docs/前端UC投影指令与数据清单-最新代码分析-2026-07-09.md`](docs/前端UC投影指令与数据清单-最新代码分析-2026-07-09.md)：只用于迁移前字段、HTTP/WS 输入与兼容风险核验，不是当前 MessageV3 真源，也不是 SessionStart 必读。
- **🟢 当前 30 Gate 机器真源** → [`../../specs/047-im-event-gate-inventory/contracts/gate-migration.v1.json`](../../specs/047-im-event-gate-inventory/contracts/gate-migration.v1.json)：Gate 分母、领域 owner、独立测试 target、删除清单、生成文件与迁移状态。
- **🟢 MessageV3 出站 schema** → [`../../specs/047-im-event-gate-inventory/contracts/message-v3-event.schema.json`](../../specs/047-im-event-gate-inventory/contracts/message-v3-event.schema.json)：业务信封只含 `event` 与 `data`；字段必须来自真实 Gate 测试及本地/远端事实。
- **⚪ 历史 Matrix 与投影文档** → `contracts/strict-matrix.v1.json`、`contracts/command-api.v1.json`、`docs/Strict Matrix/`、`docs/projection-schema.md` 已标记 `RETIRED_HISTORICAL`，只供迁移审计，不得作为当前实现或完成证据。
- **⚪ UC 自动化编排旧谱** → [`docs/uc-automation/uc-test-order-spec.md`](docs/uc-automation/uc-test-order-spec.md)：文件已标记 `SUPERSEDED`，只供迁移前依赖序追溯；当前 Gate 顺序、target 和产物入口以 Spec 047 manifest、`tests/message_v3/README.md` 与当前 Cargo 注册为准。
- **⚪ outbound 请求体历史抓包** → [`docs/csesapi-migration/真机curl真源.md`](docs/csesapi-migration/真机curl真源.md)：保留现网抓包作为兼容风险参考；当前命令体必须回到 `src/outbound/`、源码测试与当前运行证据复核，本文不是 MessageV3 完成证据。
- **⚪ csesapi 迁移全量图谱历史快照** → [`docs/csesapi-migration/full-map/00-README-index.md`](docs/csesapi-migration/full-map/00-README-index.md)：目录已标 `RETIRED_HISTORICAL`，只作旧链追溯；当前接口/事件以代码、Spec 047 和当前测试为准。
- 根 AGENTS.md §4.5.1 已挂同一索引（每次对话自动加载）。

## 本 crate 不变量

1. **只依赖 helix-core**：Cargo.toml 永不出现 tokio/sqlx/reqwest（clippy deny 兜底）
2. **handle 同步纯函数**：副作用只经 `out.push(Effect::…)`；异步走 corr 回调
3. **行为真源**：与现网行为有争议时，`cses-client/src-tauri/src/features/im/` 对应 handler 为准
4. **外部输入零 unwrap**：WS 帧 / sync 响应一律 Result（unwrap_used=warn 盯着）
5. **gate 机制铁律**：peek→落库→commit 三阶段；cursor 单调不回退；too_long 清内存不清表

## DDD 包分层地图（改代码先定位职责）

| 目录 | 职责 | 怎么用 | 依赖边界 |
|---|---|---|---|
| `acl/` | 接入层语义转换与 sync effect 映射 | 只放 wire/投影之间的 ACL 适配，不承载存储或调度策略 | 可依赖 `state/`、`schema/`、`Effect` 类型；不依赖 driver |
| `channel/` | channel gate、生命周期、older context 与写模型 | channel 状态推进、成员/帖子写入程序先找这里 | 可被 `module/`、`sync/`、`ws/` 调用；不反向依赖 outbound |
| `send/` | 发送状态机、pending、echo reconcile、pong 补偿 | 发消息乐观状态和回声对账先找这里 | 可依赖 `state/`、`channel/` 基础类型；不做 HTTP 组包 |
| `sync/` | sync 响应解析、增量哈希、补拉调度与会话状态 | cursor、水位、补偿拉取策略先找这里 | 可产出 `Effect`/落库计划；不直接 I/O |
| `query/` | 投影查询、local-first、render-ready 组装、读命令 relay | UI 读侧投影、最近消息 coverage/remote fallback 和查询结果 emit 先找这里 | 远端只产 `Effect::Http` 并复用 outbound wire 真源；不直接 I/O、不在 driver 复制 URL/merge/shape |
| `todo/` | TODO/mention/urgent 聚合与投影 emit | 待办入口和增量触发聚合先找这里 | 依赖 `state/` 与投影 schema；不改 channel gate |
| `outbound/` | HTTP 出站命令注册、组包、读写标记 | 新增/修正 csesapi 命令体先找对应子域：`posts/`、`channel_admin/`、`engagement/`、`user_misc/` | 只构造 HTTP intent；不处理 WS echo 和本地落库 |
| `ws/` | WS frame、parser、registry、handlers | 新增 live 事件先过 `ws/parser/` + `ws/handlers/` + registry | 只处理入站帧到状态机/effect；不直接调用 driver |

## 测试 pattern（新逻辑必须配回放测试）

- 用 `tests/common.rs` 的剧本工具：构造 ImModule → 喂 Tick 序列 → 断言 Effect 序列
- 乱序/边界场景进 `tests/proptest_gate.rs` 性质测试
- 回归测试命名锚定待办编号：`tests/` 内以 `// 回归锚: E{N}/B{N}` 头注释标记（Bun 对齐 3.3）
- scan 回报解析必须对齐 driver 的 rows→bytes JSON 约定（engine_loop.rs `rows_to_reply_bytes`）

## 改动验收

```bash
cargo test -p helix-im                      # 全量（45+）
cargo test --release -p helix-im --test stress_test -- --test-threads=1   # 承压（改热路径时）
```

## 系统通知的发起人身份与未读语义（必须遵守）

- 新增或修改任何会产生系统通知的业务操作，必须区分三个角色：消息发送者（可能是系统账号）、实际发起人（actor/operator）、操作对象（target/users/新群主等）。不得用系统发送者、对象用户、显示名或“您”字样推断发起人。
- 用户触发的操作必须记录服务端认证得到的稳定发起人 ID，并保证它在持久化、实时推送、同步补拉、历史查询和客户端投影中语义一致。自动任务应明确为系统发起；旧数据缺失或字段冲突按未知处理，不得猜成本人。
- 已确认由当前用户发起的操作通知仍须显示和保留；它本身不得新建“新消息”分割线、增加客户端新增消息提示数或中断原有追底。不得清除/移动已有未读边界，不得把正在读历史的用户强拉到底。其他人或身份未知的通知保留既有未读策略，禁止一概忽略 NOTICE。
- 该规则不等于修改服务端普通未读计数，也不等于替用户提交成员已读回执。发送者身份、操作人身份、阅读位置、普通未读、逐条已读必须分别处理。
- 排查先核对当前客户端命令、实际 HTTP 路由、服务端入口及实时/历史投影证据；旧方法存在不代表当前链路调用它。原来不产生通知的操作，不因本规则擅自增加通知。
- 回归至少覆盖本人/他人/未知发起人、操作对象恰为本人、历史缺字段或冲突、已有未读边界、历史阅读和混合到达批次。桌面行为须有真实 Tauri 证据；模拟测试不能代替。不得逐条通知拉用户目录或扫描全群成员。
