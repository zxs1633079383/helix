# Command Driver-Host 适配矩阵

## 生命周期登记表

每个新 Command 先填完这张表，再写实现。`N/A` 也要显式填写。

| 项目 | 要回答的问题 |
|---|---|
| Command/入口 | 名称、payload/schema、公开 `Tick::Command` 或 handler seam、调用方是什么？ |
| 分类 | 本地确定性、远程读、远程写、WS/事件驱动？ |
| 首批 Effect | 注入后第一批 `Persist`/`Http`/`Send`/`ScheduleTimer`/`Emit` 是什么？ |
| 关联 | `corr`、`req_id`、temporary id 在哪里注册、回灌、幂等匹配？ |
| 中间态 | optimistic 状态、pending、timer、transport 状态是什么？ |
| 外部边界 | 是否真的触达 Go HTTP、Go WS、SQLite、EventSink？需要哪一级证据？ |
| 成功终态 | 哪一个 `DomainEvent`/result、durable row 或确定性 Effect 才算成功？ |
| 失败终态 | 非法输入、未知命令、HTTP/WS、业务 status、Persist、timeout、取消各是什么？ |
| 清理 | 成功/失败/超时/取消后哪些 session、WS、timer、pending、临时数据必须释放？ |

## 共享 adapter 选择

| Command 形态 | 首选入口 | 已能证明 | 不能替代 |
|---|---|---|---|
| 本地确定性 | `crates/helix-im/tests` focused seam | Effect、事件、错误、幂等 | 真实 Go/Native/SQLite 链 |
| 远程读 | `shared::assert_real_command_event` | Native session、Go HTTP、PortReply、终态事件、证据、关闭 | 特殊 WS 推送或自定义业务终态，需自己的 predicate |
| 标准 TEXT 发送 | `shared::assert_isolated_send` | run-owned 频道、`sending → post → received → sent`、SQLite、EventSink、清理 | Pin/Search/Todo 等非标准发送流程 |
| 自定义远程写 | `start_real_helix_session` + `RealHelixSession::command`/`wait_event`/`close` | 可逐跳观察 Effect、PortReply/WS、projection、终态、清理 | 自动猜测业务成功；必须写明确 predicate |
| WS 入站 | 与 actor Helix session 分开的 raw observer | 原始帧到达、帧字段和本轮 correlation | Helix 已消费、持久化、Emit 或 UI 已渲染 |

共享实现位置：

- Native test harness：`crates/helix-driver-native/tests/messageV3-domain/shared/mod.rs`；
- Native composition root：`crates/helix-driver-native/src/engine_loop.rs`；
- 通用 Effect dispatch：`crates/helix-driver-host/src/dispatch.rs`；
- HTTP/WS/SQLite adapters：`crates/helix-driver-host/src/network/`、`network.rs`、`storage.rs`；
- Command 业务真相：`crates/helix-im/src/commands.rs`、对应 module/flow/port-reply 文件。

如果 adapter 不足，优先在 `shared/mod.rs` 增加一个参数化 helper，并用 focused test 锁住 helper 的边界；不要在单个 Gate 中复制 transport、DB 或 engine loop。

## G01a 标准链

G01a 是标准 TEXT 发送的参照，不是所有 Command 的通用语义：

```text
Tick::Command(send)
  → optimistic im:post:sending + SQLite send_status=sending
  → helix-driver-host HTTP /posts/create
  → Go WS action=post
  → Helix WS inbound / projection
  → im:post:received
  → SQLite send_status=sent
  → EventSink bus terminal event
  → session/WS/engine/临时频道清理
```

对 G01a，至少应关联同一 run 的 command、HTTP、WS、SQLite 和 EventSink 证据；不能只断言返回 200 或某个原始 WS observer 收到帧。

## 证据等级

| 等级 | 证据边界 | 典型断言 |
|---|---|---|
| `FOCUSED` | `helix-im` 公开 seam | 首批 Effect、错误、幂等、终态事件 |
| `LOCAL_INTEGRATION` | host/SQLite/transport 本地组合 | dispatch、PortReply、durable row、EventSink |
| `REAL_NATIVE_CHAIN` | Native engine + 真实 Go HTTP/WS + Helix projection + SQLite/EventSink | 外部边界、回灌、持久化屏障、唯一终态、清理 |
| `UI_DOM` | 客户端宿主最终渲染 | canonical projection 到 UI 的显示与交互 |

等级之间不可互换：HTTP 200、健康检查、静态 manifest、编译通过、截图和 EventSink 单点都不能单独升级为 `REAL_NATIVE_CHAIN`。

## 远程读的最小形态

```rust
shared::assert_real_command_event(
    "PhaseX",
    "Read",
    "MV3-Gxx",
    "im:read:result",
    resource_name("read"),
    json!({ /* command payload */ }),
    |body| body.get("status").and_then(Value::as_str) == Some("SUCCESS"),
).await?;
```

实际参数和终态 predicate 以当前 helper 签名、Gate contract 与源码为准；`im:read:result` 的 Go body 校验是该 helper 的特例，其他终态不要盲套。

## 自定义远程写的最小形态

```rust
let session = shared::start_real_helix_session(/* run-owned config */).await?;
session.command("command-name", payload).await?;
let terminal = session.wait_event("im:domain:result", timeout).await?;
assert_eq!(terminal.corr, expected_corr);
assert_sqlite_terminal_row(&session, expected_corr).await?;
session.close().await?;
```

真实测试必须补齐：失败/timeout/取消路径、重复/迟到回灌、Persist barrier、run-owned 清理和 evidence 写出。若 `close` 自身失败，结果应标 `CLEANUP_FAILED`，不能因业务断言通过而宣称 Green。
