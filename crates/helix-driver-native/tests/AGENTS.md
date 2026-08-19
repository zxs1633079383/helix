# helix-driver-native 测试指令：Command 生命周期与真实链

本目录是 Native driver 的集成测试边界。它验证 `helix-core`、`helix-im`、Native/Host driver、本地 SQLite、EventSink 与真实 Go HTTP/WS 的组合行为；不验证 Angular DOM，也不能把纯 replay、mock server、health 200 或单个 HTTP 2xx 写成真实业务 PASS。

## 1. TDD 前置

每新增一个 Command，必须先在 `crates/helix-im/tests` 或本目录对应的公开 seam 写失败测试，再实现生产代码。测试必须从调用者可见的入口开始：

```text
Command payload
  → Tick::Command/AppCommand
  → ImModule/ExecutionShell
  → Effect
  → PortReply 或 Tick::Inbound
  → projection/read-back
  → terminal DomainEvent/result
```

禁止只断言“产生了一个 `Effect::Http`”就宣称 Command 生命周期完成。每条测试在提交前必须有：

- 红测试记录：明确哪个生命周期不变量先失败；
- 绿测试：成功终态和关键中间态；
- 失败测试：至少覆盖输入/端口/持久化/超时中一个与该 Command 相关的失败边界，远程写命令不得缺端口或持久化失败；
- 幂等测试：重复、迟到或重放响应不会重复终态事件、重复写入或遗留 correlation；
- 清理测试或显式 read-back：pending、timer、`corr_map`、WS/engine 和本轮 SQLite 在结束后收敛。

## 2. Command 类型与测试落点

| Command 类型 | 最低测试落点 | 必须证明 |
|---|---|---|
| 本地确定性 | `crates/helix-im/tests` | 输入校验、Effect/DomainEvent、失败与幂等；不需要伪造 Go |
| 远程读 | `crates/helix-im/tests` + 必要时本目录 | HTTP 请求契约、真实 PortReply、`status=SUCCESS`、`req_id/corr`、`im:read:result` 或约定 result 终态 |
| 远程写/发送 | `crates/helix-im/tests` + `messageV3-domain/Phase*/Domain*/Gate*/` | optimistic → HTTP/WS → Persist/PersistAtomic → read-back → terminal event；失败不得假装 received |
| WS/事件驱动 | `crates/helix-im/tests` + 本目录 | `Tick::Inbound`、事件解析、Gate/序号/权限、持久化屏障和 Emit 顺序 |

测试文件、Gate 入口和 Cargo target 必须与当前注册保持一致：

- `message_v3_domain.rs`：Gate 模块注册；
- `messageV3-domain/Phase*/<Domain>/<Gate>/main.rs`：独立真实链入口；
- `messageV3-domain/shared/mod.rs`：共享资源、engine、端口、断言和证据；
- `Cargo.toml`：test target；
- `crates/helix-im/tests`：sans-IO/Command focused 生命周期测试。

## 3. 标准八步生命周期

每个触达 Native/Go 的 Command 都按下面八步实现或明确标记 `N/A`。G01a 是 TEXT send 的参考实现；新增 Command 不得只复制文件名，必须替换为自身的终态和证据字段。

### 第 1 步：定义场景与输入

- 固定 command 名称、payload、身份、resource/run token 和预期终态；
- 先写 focused red test；
- 禁止从测试内部直接调用 private helper 冒充公开 Command。

### 第 2 步：准备前置与 run-owned 资源

- 远程命令：真实 Go health、账号、租户、权限和资源 fixture；
- 本地命令：确定性 Clock/Id/Storage/Transport；
- 缺少真实前置必须标记 `BLOCKED_PRECONDITION`/`NOT_VERIFIED`，不能降级成 PASS。

### 第 3 步：启动 Native 生命周期

按 Command 实际需要启动 `NativeHttp`、`NativeTransport`、`NativeStorage`、`NativeEventSink`、`ExecutionShell` 和 engine loop；等待连接建立事件，禁止在 engine 未 ready 时注入命令。

### 第 4 步：从公开入口注入 Command

使用 `Tick::Command(AppCommand::new(...))` 或项目约定的公开入口，并记录 command/correlation/temporary id。测试不得直接调用 `handle_send_message` 等 private 方法。

### 第 5 步：验证第一批 Effect 与本地中间态

验证 Command 的第一批 Effect 与契约一致，并验证所有必须的 optimistic/pending 状态已经持久化。例如 G01a 必须观察：

```text
im:post:sending
message.send_status == "sending"
```

只看到 Effect、只看到 HTTP admission 或只看到本地 row，都不能算终态。

### 第 6 步：验证真实外部边界

远程命令必须观察真实 HTTP/WS 边界：

- HTTP：请求 method、endpoint、身份头、关键 body、HTTP 状态和业务信封；
- WS：hello/目标 action、关联 id、方向和必要字段；
- 禁止把 fake server、静态 fixture、health 200 或单独 HTTP 200 当作业务完成。

G01a 的外部事实是 Go `/channel/create`、`/posts/create` 和 WS `action=post`；observer raw WS 是线帧证据，actor NativeTransport 才是 Helix 入站证据。

### 第 7 步：验证 PortReply、投影、持久化与终态事件

每个异步 correlation 必须有对应回灌和终态处理。远程写命令通常至少需要：

```text
PortReply/Inbound
  → Gate 或业务状态机
  → Persist/PersistAtomic
  → PersistOk/read-back
  → terminal DomainEvent/Result
  → NativeEventSink
```

G01a 的最低终态断言是：

```text
WS post.temporaryId == sending.temporaryId
im:post:received.temporaryId == sending.temporaryId
message.send_status == "sent"
bus.channel == "im:post:received"
```

成功事件不得早于 required PersistOk/read-back；失败路径不得发成功事件。

### 第 8 步：写证据并清理

证据至少包含：

- `run_id`/resource、command、correlation/temporary id；
- 关键 Effect 或 HTTP/WS hop；
- 中间态与终态持久化 read-back；
- terminal event/result 与 EventSink channel；
- status、失败分类、前置缺口和清理结果。

结束时必须关闭 engine/transport、删除本轮临时 SQLite、关闭本轮远程资源；清理失败必须使测试失败。证据文件写入 `artifacts/messagev3-phase1-4-real-chain/<run_id>/...`，不得写入 secret、cookie 或完整凭证。

## 4. 生命周期完成标准

一个新 Command 只有同时满足以下条件才可标记完成：

1. focused TDD 测试先红后绿；
2. 成功、失败、超时/重复边界已按命令类别覆盖；
3. 所有 correlation/pending/timer 在终态清理；
4. 远程命令具备同轮 HTTP/WS/本地持久化/EventSink 证据；
5. 本地命令具备确定性 Effect/DomainEvent/read-back 证据；
6. `git diff --check` 与对应聚焦测试通过；
7. 未验证的 Go、WS、SQLite、UI 或远端 DB 能力显式标记 `PARTIAL`/`BLOCKED`，不得用相邻 Gate 证据替代。

## 5. 反模式

- 先写生产 Command、最后补测试；
- 只断言 HTTP 200、`accepted`、health 或静态注册；
- 只断言 `Effect::Http`，不等 PortReply/Inbound 和终态；
- 测试直接调用 private handler，绕过 `Tick::Command`；
- 只检查内存状态，不做 SQLite/持久化 read-back；
- 用 observer raw WS 代替 Helix actor 入站，或用本地 EventSink 代替真实 Go；
- 用 mock/fixture/replay 证据宣称 real Go/native/UI PASS；
- 忽略 cleanup、timer、pending 或 correlation 泄漏。
