---
name: messagev3-cross-layer-emit-diagnosis
description: 沿 Angular → Ipc → Tauri/driver → Helix Tick/Effect → Go HTTP/WS → Persist/Projection → EventSink → Angular 的真实调用链，定位 MessageV3 事件或字段在哪一跳丢失，并以 TDD、逐跳证据和分层验收指导修复。用户说“WS 有值但 Angular undefined”“emit 没出去”“source/type 丢了”“新 Command 生命周期不完整”或需要开发/调试 MessageV3 跨层功能时使用。
---

# MessageV3 跨层 Emit 排查与开发

## 目标与触发

本 Skill 处理的是“事实已经在某一层，但下一层没有看到”的问题，也用于新增或修改会穿过外部 HTTP/WS、SQLite、EventSink 和 Angular 的 MessageV3 Command。目标不是猜测哪一层有 bug，而是用同一个 `corr`/`req_id`/`channelId`/`event`/`seq` 在相邻边界逐跳对账，找到第一处不再相等的边界，然后只修该边界的 owner。

典型触发语句：

- “Go WS 的 `channel_created` 有 `source.type`，Angular 却是 `undefined`。”
- “日志显示收到事件，但 `IpcEventHandlerService` 里的对象缺字段。”
- “emit 是否没提交？当前实际用哪个 EventSink？”
- “新 Command 如何从 `Tick::Command` 走到真实 Go 和最终事件？”
- “这条 MessageV3 事件是 projection 丢的、Tauri bus 丢的，还是 Angular 合并丢的？”

## 不可违反的诊断原则

1. 先读当前源码、分支、配置和运行态，再下结论；历史记忆只用于导航，不能替代当前证据。
2. 把每个边界当成独立证据面：原始 Go WS、Helix 入站、`Effect::PersistAtomic`、持久化读回、`Effect::Emit`、Native sink、Tauri `im:__bus__`、Angular adapter/store、DOM 不能互相冒充。
3. 只修第一丢失点。Angular 不补造 Helix 业务事实，driver 不重建 IM projection，Go WS 有字段也不等于 Helix 已消费、已持久化或已 Emit。
4. `HTTP 200`、健康检查、编译成功、静态 manifest、截图或“收到了某个事件”都不能单独称为真实链路 PASS。
5. 保留用户已有 dirty/untracked 文件；测试清理只删除本轮 run-owned 资源，禁止 truncate 共享库或用手工改库制造成功。
6. 涉及写路径时遵守 `helix-hot-path-write-audit`：固定字段复制可接受，但不要整对象 read-modify-write、无界遍历或把临时 debug clone 扩散到热路径。

## 开局与范围固定

执行诊断或改代码前：

```bash
python3 ~/.codex/scripts/log_preflight.py
git status --short --branch
```

然后读取就近 `AGENTS.md`，至少确认 `helix-im`、`helix-driver-host`、`helix-driver-native/tests` 和客户端相关目录的约束。跨 crate 或新增 harness 时，先读 `docs/GOAL.md`、`docs/ROADMAP.md`、`crates/helix-im/docs/projection-schema.md`；若触及 Native 真实链，再读 `crates/helix-driver-native/tests/AGENTS.md`。不要先启动一堆服务再反推代码。

在笔记或报告开头登记本轮：

```text
run_id=<本轮运行标识>
corr/req_id=<关联键，缺失则说明>
channel_id=<业务实体>
expected_event=<例如 im:channel:created>
expected_fields=<例如 source.type=createAt=createBy>
```

没有关联键时先补观测或使用唯一的 `channelId + event + seq` 组合；不要把不同轮次的日志拼成一条链。

## 标准逐跳链

命令和事件的具体名称可能不同，但默认按下面的顺序找 owner；每一箭头都要有源码锚点或本轮运行证据：

```text
Angular action
  → MessageV3 Ipc/Port command adapter
  → Tauri invoke / IPC bridge
  → Tick::Command
  → helix-im command handler
  → Effect::Send / Effect::ScheduleTimer
  → Go HTTP response 或 Go WS action
  → Helix WS handler / PortReply dispatcher
  → queue_*_persist
  → Effect::PersistAtomic
  → PortOutcome::Ok / PersistOk
  → projection builder
  → event::<domain>::created/updated(...)
  → MessageV3Event::into_effect()
  → Effect::Emit
  → helix-driver-host dispatch
  → NativeEventSink / FFI/Web EventSink
  → Tauri app.emit_str("im:__bus__", ...)
  → Angular IpcEventService
  → IpcEventHandlerService / adapter / store
  → component or DOM
```

建群类事件的当前源码地图和检索锚点见 [`references/source-map.md`](references/source-map.md)。源码已漂移时以 `rg`/图谱重新定位，不要照抄旧行号。

## 1. 先建立契约，不先改代码

记录四类事实：

- **入口**：Command 名称、payload/schema、调用方、公开 seam（`Tick::Command`、handler 或 port）。
- **预期终态**：唯一的 `DomainEvent`/result、持久化行、cursor/seq 和必须保留的字段。
- **失败终态**：非法输入、未知命令、HTTP/WS 错误、业务 `status != SUCCESS`、Persist 错误、timeout、取消。
- **生命周期**：`corr_map`、pending、timer、WS/worker 的注册、回灌、重复/迟到响应和释放。

把字段合同写成相邻边界表，而不是只写“最终应该有值”：

| 边界 | 应观察什么 | 最低证据 |
|---|---|---|
| Go → Helix | 原始 action、data、`corr/seq`、字段类型 | raw WS frame/log |
| Helix 入站 → Persist | handler 命中、StorageOp、同一 `channelId` | focused/host log |
| Persist → Projection | `PortOutcome::Ok`、读回值、projection JSON | SQLite/PortReply |
| Projection → Emit | `{event,data}`、字段完整性、Emit 次数 | EventSink receiver |
| Native/Tauri → Angular | bus channel、序列化文本、监听器 | Tauri/IPC log |
| Angular → UI | adapter 输出、upsert 合并、最终 view model/DOM | focused/UI test |

## 2. 找第一丢失点

对同一字段逐跳做“存在/类型/值”比较。例如 `source.type` 必须分别记录：

```text
raw_ws.data.source.type          = "meeting"
helix_channel_input.source.type  = ?
sqlite/readback.source.type      = ?
projection.source.type           = ?
event.data.source.type           = ?
bus_payload.data.source.type     = ?
angular_adapter.source.type     = ?
handler/view_model.source.type  = ?
```

按下面规则定位：

- raw WS 有值，Helix 输入没有：协议解析、WS handler 或上游 payload 适配问题。
- Helix 输入/SQLite 有值，projection 没有：`build_*_projection` 或事件 builder 丢字段；优先在该函数加 focused regression test。
- projection/EventSink receiver 有值，Tauri bus 没有：host/native sink、序列化或 bridge 问题。
- Tauri bus 文本有值，Angular adapter 没有：adapter 解 envelope、字段映射或类型收窄问题。
- adapter 有值，handler/view model 没有：`undefined` 过滤、upsert 合并、scope 过滤或客户端转换问题。
- created event 缺字段但 durable query/read-back 有字段：这是“创建事件投影合同”和“持久化读回合同”不一致，不能把 Go WS 或 SQLite 误判为丢值。

每次只扩大一个边界。第一处失败保留原始日志和断言；不要靠重跑覆盖首次失败。

## 3. TDD 修复顺序

### 红：在第一丢失 seam 写最小可证伪测试

测试必须先证明问题真实存在，再写生产修复：

1. 优先从公开 `Tick::Command`/handler seam 进入；如果丢失点是一个纯 projection helper 且没有更近的公开 seam，可直接测试该确定性 helper，但要在报告中注明这是 `FOCUSED`，不能冒充真实链。
2. 断言字段值和类型（例如 `projection["source"]["type"] == "meeting"`），不要只断言对象存在或 Emit 次数为 1。
3. 同时为非法输入、远端/业务失败、Persist 错误、timeout/取消、重复/迟到回灌和 cleanup 保留可证伪断言；不适用的阶段写 `N/A`。

### 绿：只修 owner 层

常见修复位置：

- projection builder：复制权威字段到 `{event,data}`，保持字段命名、类型和边界，不把完整 Go 对象无条件透传。
- `EventSink`/host：修复事件 envelope、序列化或 receiver 生命周期，不把业务 reducer 放进 driver。
- Angular adapter：只做协议转换和类型校验，不在客户端推导 seq、权限、未读或 source 业务语义。

固定字段复制应是有界 O(1)；避免 N 次 JSON read-modify-write、无界递归和重复整对象 clone。

### 重跑：先 focused，再真实链

```bash
# 根据当前 crate/test target 替换过滤器
cargo test -p helix-im --lib <focused_filter> -- --nocapture
cargo test -p helix-driver-native --test message_v3_domain <gate> -- --nocapture --test-threads=1
git diff --check
```

触达外部服务的 Command，只有同一轮同时拿到 HTTP/WS、Helix projection、SQLite/持久化、EventSink 和客户端证据，才可标记 `REAL_NATIVE_CHAIN`；否则写 `FOCUSED`、`LOCAL_INTEGRATION` 或 `UNVERIFIED`。

## 4. 新 Command 的开发接入

新增 Command 时将本 Skill 与 `$helix-command-driver-host-test` 配合使用：前者负责“字段/事件在哪一跳丢失”的诊断，后者负责 G01a 八步、共享 adapter、真实 Go/Native 生命周期和 run-owned 清理。两者不互相替代。

最低生命周期清单：

1. 有效输入从公开入口进入并产生第一批 Effect；
2. 非法输入/未知命令 fail-closed，无伪成功和 pending 残留；
3. 每个异步 Effect 都有 PortReply/WS 回灌，只有真实终态才 Emit；
4. HTTP/WS、业务 status、Persist、timeout/取消各有失败断言；
5. 重复、迟到、断线恢复不重复 Emit、不重复写入、不倒退状态；
6. 成功、失败、timeout、取消后 `corr_map`/pending/timer/transport 无泄漏；
7. 真实边界保留 raw HTTP/WS、durable state、projection、EventSink 和客户端证据；
8. 清理只删除本轮资源，并记录清理结果。

## 5. 证据报告合同

交付或评论必须明确：

```text
结论：FIXED | PARTIAL | BLOCKED | UNVERIFIED
第一丢失点：<file:line + symbol + observed/expected>
调用链：<入口 → ... → 最终边界>
改动：<只列 owner 文件和字段/协议影响>
FOCUSED：<命令 + 红/绿结果>
LOCAL_INTEGRATION：<若运行，列 host/storage/transport 证据>
REAL_NATIVE_CHAIN：<run_id/corr/HTTP/WS/SQLite/EventSink/client 证据；没有就写未运行>
UI_DOM：<若运行，列 viewport/状态/证据；没有就写未运行>
未验证/阻塞：<真实 Go、认证、WS、SQLite、Tauri 或环境缺口>
清理：<run-owned 资源与结果>
```

失败分类使用：

- `CONTRACT_ERROR`：payload、event、字段类型或终态合同与源码不一致；
- `PROJECTION_LOSS`：权威输入/持久化有值，projection/event data 丢值；
- `BRIDGE_LOSS`：EventSink 有值，Tauri/IPC bus 丢值；
- `CLIENT_ADAPTER_LOSS`：bus 有值，Angular adapter/store 丢值；
- `REAL_CHAIN_FAILED`：真实边界已启动，但请求、回灌、持久化或终态失败；
- `BLOCKED_PRECONDITION`：Go、认证、WS、SQLite、Tauri 或测试宿主未就绪；
- `CLEANUP_FAILED`：本轮资源未释放；
- `UNVERIFIED`：只有静态或 focused 证据，不能声称真实链 PASS。

## 关键反模式

- 看到 Go WS 有 `source` 就直接改 Angular；先证明 Helix projection 是否已经有它。
- 看到 `to_effect_s1::emit_channel_created` 就假定它是当前 emitter；先用 `rg` 找调用点和真实调用者。
- 只看 `IpcEventHandlerService` 的 console 对象；开发者工具可能折叠或延迟展示对象，必须在边界保存结构化快照。
- 用 `source?.type ?? 'meeting'` 等客户端默认值遮盖上游合同缺口。
- 用健康 200、EventSink 收到任意一条事件、或 UI 截图替代同轮的 durable/projection/cleanup 证据。
- 为每个 Gate 复制一套 HTTP、WS、SQLite、engine loop；优先扩展共享 adapter，并保留各 Gate 的业务 predicate。

## 参考资料

- 当前源码锚点和 G01a 对账入口：[`references/source-map.md`](references/source-map.md)
- 新 Command 的 driver-host/Native 生命周期：[`$helix-command-driver-host-test`](../helix-command-driver-host-test/SKILL.md)
- 热路径写入审计：[`../helix-hot-path-write-audit/SKILL.md`](../helix-hot-path-write-audit/SKILL.md)
