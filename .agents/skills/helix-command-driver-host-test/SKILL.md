---
name: helix-command-driver-host-test
description: 为 Helix 新增或修改业务 Command 接入 helix-driver-host 与 Native/真实 Go 生命周期测试。需要补 Command 的 TDD、Tick→Effect→PortReply/WS→Persist/read-back→terminal event、G01a 类 real-chain、driver-host 适配、证据和清理时使用。
---

# Command Driver-Host 测试接入

## 目标

把新 Command 接入现有的 `helix-driver-host`/Native 测试装配，而不是为每个 Gate 复制 HTTP、WebSocket、SQLite、EventSink 或 engine loop。业务事实仍由 `helix-im` 计算；host 只执行 Effect，native 只负责真实平台组合与测试观测。

本 Skill 解决“Command 已能编译，但生命周期没有被证明”的问题：从公开 seam 的红测开始，经过真实 PortReply/WS 回灌、持久化读回和终态事件，最后给出分层证据与 run-owned 清理。

## 何时使用

遇到以下任一情况就使用本 Skill：

- 新增或修改业务 `Command`，并需要 Native/Go/driver-host 测试；
- 新增 MessageV3 Gate，或要把 Gate 接到 `crates/helix-driver-native/tests/messageV3-domain`；
- 需要验证 Command 的 HTTP、WS、SQLite、EventSink、幂等、超时、取消或清理生命周期；
- 需要复用 G01a 的共享 session、真实频道、证据和清理 helper。

以下任务不由本 Skill 覆盖：只改 `helix-driver-host` 的通用 I/O 实现、只改 Go API、只做 Angular DOM 验收，或只做不触及平台端口的纯 sans-IO replay。它们可以分别交给对应 crate 的就近规则。

## 开局检查

按顺序执行：

1. 运行 `python3 ~/.codex/scripts/log_preflight.py`，记录任务摘要；涉及三个以上文件或新增 harness 文档时，在 `SESSION.md` 写可证伪的 `🎯 EXPECT`。
2. 阅读 `docs/GOAL.md`、`docs/ROADMAP.md`、最近的 `AGENTS.md`，以及 `crates/helix-driver-native/tests/AGENTS.md`。后者是 Native G01a 八步和证据等级的就近真源。
3. 先用 codebase-memory 的 `search_graph`/`trace_path` 定位 Command、`ExecutionShell`、native composition root 和 host dispatch；图谱不足时再用 `rg`。不要凭文件名猜调用链。
4. 查看 `git status --short --branch`，保留既有 dirty/untracked 文件；只把本次 Command 接入的文件放入交付范围。
5. 读取本 Skill 的 [`references/command-adapter-matrix.md`](references/command-adapter-matrix.md)，选择已有 adapter；不要从 G01a 测试复制整段 setup。

## 标准工作流

### 1. 先写生命周期契约

在测试或 Gate 说明中先登记：

- Command 名称、payload/schema、公开入口和调用方；
- 分类：本地确定性、远程读、远程写或 WS/事件驱动；不适用的阶段写 `N/A`；
- 第一批 Effect、`corr`/`req_id`/临时 ID 注册点和回灌点；
- 成功终态：`DomainEvent`/result、持久化状态或确定性 Effect；
- 失败终态：非法输入、未知命令、HTTP/WS/业务失败、Persist 错误、timeout、取消；
- 重复/迟到/断线回灌的幂等规则；
- 成功、失败、超时、取消后的 pending、timer、`corr_map`、worker/WS 清理；
- focused、local integration、real Native/Go chain、UI/DOM 各自能证明什么。

### 2. TDD 红测先行

在公开 seam 写会失败的 focused test，再实现最小代码：

1. 有效输入能从公开入口进入并产生契约规定的首批 Effect；
2. 非法输入和未知命令 fail-closed，不伪造成功、不遗留 pending；
3. 每个异步 Effect 都有 PortReply/Inbound 回灌，只有到真实终态才 Emit success/result；
4. HTTP/WS/业务失败、Persist 失败、timeout 至少各有可证伪断言；
5. 重复、迟到、断线恢复不重复 Emit、不重复写入、不倒退状态；
6. 成功、失败、超时、取消都无资源泄漏。

focused 测试应通过 `Tick::Command` 或公开 handler seam 进入，不能直接调用 private handler 来制造假绿。先运行红测，确认失败原因是缺实现/缺契约，再实现当前切片并重跑成功与失败路径。

### 3. 选择共享 adapter

- 本地确定性：放在 `crates/helix-im/tests`，只证明 core/module 的 Effect、事件、错误和幂等。
- 远程读：优先 `shared::assert_real_command_event`，它负责 Native session、真实 Go HTTP、终态事件、证据和关闭。
- 标准 TEXT 发送：优先 `shared::assert_isolated_send`，它负责 run-owned 真实频道及 G01a 标准链。
- 自定义远程写或特殊 WS：使用 `start_real_helix_session`、`RealHelixSession::command`、`wait_event`、`close`，只补业务 predicate；不要重新实现 engine loop。
- 现有 adapter 缺能力时，先在 `messageV3-domain/shared/mod.rs` 增加通用 helper 和 focused 覆盖，再让 Gate 调用；禁止每个 Gate 搭一套 transport/DB。

### 4. 真实链按八步验收

对触达外部服务的 Command，按就近 `AGENTS.md` 的 G01a 模板逐步取得证据：

1. 注入公开 Command，确认入口与 payload；
2. 确认第一批 Effect、`corr`/`req_id` 和 optimistic 状态；
3. 确认 `helix-driver-host` 执行 HTTP/WS/Storage/Emit，而非测试旁路；
4. 确认真实 Go HTTP/WS 边界和响应/推送匹配本轮 correlation；
5. 将 PortReply/WS inbound 回灌 Helix，确认业务状态机继续推进；
6. 等待 `PersistOk` 或等价持久化屏障，并读回本轮 SQLite 行/状态；
7. 确认 EventSink 收到唯一终态 `{event,data}`，必要时确认 Go/远端事实；
8. 关闭 session、WS、engine，校验 run-owned 资源清理并写出 evidence。

健康检查 200、静态 K6 manifest、HTTP 200、编译通过或截图都不能单独证明业务成功。

### 5. 断言与证据分层

至少覆盖：入口参数、Effect 顺序/数量、correlation、真实 HTTP/WS 请求与回灌、Persist barrier、SQLite durable row、EventSink 终态、业务失败、timeout/取消、幂等和 cleanup。

报告中分开标记：

- `FOCUSED`：纯 `helix-im` seam；
- `LOCAL_INTEGRATION`：本地 host/SQLite/transport 组合；
- `REAL_NATIVE_CHAIN`：Native engine + 真实 Go HTTP/WS + Helix projection + SQLite/EventSink；
- `UI_DOM`：客户端渲染或宿主最终证据。

低等级证据不能冒充高等级证据；EventSink 不能冒充 Go DB/Pulsar，原始 WS observer 不能冒充 Helix 已消费、持久化和 Emit。

### 6. 注册、运行和收尾

新增 Gate 时按实际需要同步：Gate `main.rs`、`message_v3_domain.rs`、`Cargo.toml` test target、Gate contract/registry、evidence/README。仅新增复用测试不应无理由改分母。

聚焦运行示例：

```bash
cargo test -p helix-im --test <focused_test> <test_name> -- --nocapture
cargo test -p helix-driver-native --test message_v3_domain <test_name> -- --nocapture --test-threads=1
git diff --check
```

完成前交付 Command 分类、入口、调用链、adapter、红绿结果、真实 run/correlation/evidence 路径、失败首跳、清理结果和未验证项。若存在第二个无关失败，停止扩张并标为 `PARTIAL`/`BLOCKED`，不要把目标架构写成当前事实。

## 失败分类

- `CONTRACT_ERROR`：payload、事件、终态或断言与源码/合同不一致；
- `HARNESS_GAP`：共享 helper 缺能力，先扩展 shared harness；
- `BLOCKED_PRECONDITION`：Go/WS/SQLite/认证/配置等真实前置未就绪；
- `REAL_CHAIN_FAILED`：真实边界已启动，但请求、回灌、投影或终态失败；
- `CLEANUP_FAILED`：本轮 session、WS、engine、临时频道或证据资源残留；
- `UNVERIFIED`：只有静态/局部证据，不能宣称 real-chain PASS。

首次失败要保留；不要用重跑、健康 200 或手工改库覆盖它。共享库不能被测试 `truncate`，清理只能删除本轮 run-owned 资源。

## 参考

详细 adapter、证据矩阵和 G01a 示例见 [`references/command-adapter-matrix.md`](references/command-adapter-matrix.md)。生产调用链的 owner 仍以源码和就近 `AGENTS.md` 为准。
