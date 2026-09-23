---
name: messagev3-cross-layer-coding
description: 按契约冻结、TDD 红绿、逐跳字段对账和分层证据，实施或修复穿越 Angular → IPC/Tauri → Helix → Go HTTP/WS → Persist/Projection → EventSink → Angular 的通用 MessageV3 Emit 功能。用户要求新增或迁移跨层 Command、修复事件/投影字段、让 Go/Helix/driver/Angular 协同交付，或明确要求跨仓编码时使用。
---

# MessageV3 跨层 Coding

## 目标与适用范围

把一个跨层 MessageV3 需求从“字段/事件合同”推进到可验证实现。先冻结相邻边界的输入、输出、成功终态和失败终态，再按 owner 编码；每一跳都保留同一 `run_id`、`corr`/`req_id`、`channelId`、`event` 和 `seq` 的证据。

本 Skill 覆盖：

- 新增或修复跨 Go、Helix、driver/host、Tauri 和 Angular 的 MessageV3 Command 或事件；
- 任意 Gate 的绝对/增量投影、集合与计数、事件字段和实时推送；
- Go HTTP/WS wire、Helix `Tick`/`Effect`/`PersistOk`、EventSink、IPC bus 和 Angular adapter 的协同实现；
- focused、local integration、真实 Go/Native chain 和 UI/DOM 的分层验收。

纯 Angular 视觉修改、纯 Go API 修改、纯 Helix replay 或只改通用 host I/O 时，转交对应就近 Skill；需要定位“第一处丢失”时先叠加 [`messagev3-cross-layer-emit-diagnosis`](../messagev3-cross-layer-emit-diagnosis/SKILL.md)，需要 Native/Go 生命周期时叠加 [`helix-command-driver-host-test`](../helix-command-driver-host-test/SKILL.md)。

## 不可越过的边界

| 层 | 允许拥有 | 禁止拥有 |
|---|---|---|
| Go/远端 authority | HTTP/WS wire、广播 audience、远端持久事实和服务端查询结果 | 让 Angular 计算缺失 projection；用兼容字段掩盖合同漂移 |
| `helix-im` | IM 业务事实、权限、已读/未读集合与计数、顺序、幂等、恢复、PersistOk 后的 `{event,data}` | DOM、Tauri API、HTTP client、平台 fallback |
| `helix-driver-*`/host | HTTP/WS/SQLite/timer、Effect 执行、原样批量交付事件 bytes | 重建 reducer、推导未读、修改业务 payload |
| Angular/IPC | 提交最小 intent、校验 envelope、消费绝对 projection、更新 view model | 根据 `readerUserId` 做差集、补默认值、刷新旧列表、推导权限/排序/计数 |

判据：多个平台必须得到相同业务结果的规则归 `helix-im`；如何执行 I/O、如何绘制和暂存交互状态归边缘。Angular 只消费 Helix 已完成的合同字段（例如某个 read/receipt Gate 的集合与计数）；字段不存在就报告合同错误，不得兜底为旧值或本地推导值。

## 开局固定范围

按顺序执行：

1. 从任务 handoff 恢复；首次任务运行 `python3 ~/.codex/scripts/log_preflight.py`，同任务未发生相关变化时复用摘要。核对 `git status --short --branch`，保留既有 dirty/untracked，不 reset、clean、stash 或 truncate 共享库。
2. 命中根 AGENTS 的实质任务门槛时，在 `SESSION.md` 留一行可证伪 `🎯 EXPECT`；完整目标、身份、字段不变量和证据保存在 handoff，SESSION 只引用。
3. 按当前任务读取或复用已读且未变化的根 `AGENTS.md`、目标目录的就近 `AGENTS.md`、[`docs/GOAL.md`](../../../docs/GOAL.md)、[`docs/ROADMAP.md`](../../../docs/ROADMAP.md)。触碰投影、EventSink 或 bus 时再读 [`projection-schema.md`](../../../crates/helix-im/docs/projection-schema.md)。
4. 先用 codebase-memory 的 `search_graph`/`trace_path` 定位符号与调用者；图谱不足再用 `rg`。从 [`references/source-map.md`](../messagev3-cross-layer-emit-diagnosis/references/source-map.md) 取检索入口，但必须用当前源码重新确认 `file:line`。
5. 固定仓库和文件租约：记录每个 repo 的 cwd、branch、HEAD、允许修改的目录、禁止触碰的 dirty 文件和 owner。Go 只有在用户明确把 Go 纳入本轮范围时才改；否则保留精确 Go `file:line` 根因给上游。

在笔记或报告开头登记：

```text
run_id=<唯一运行标识>
corr/req_id=<关联键；缺失则用 channelId+event+seq，并说明>
channel_id=<业务实体>
command_or_action=<Command 或 Go action>
expected_event=<稳定 MessageV3 event>
expected_fields=<字段名、类型、绝对/增量语义>
```

## 1. 先冻结合同，再写代码

将需求写成一页跨层合同，至少回答：

- **入口**：Angular intent、IPC command、`Tick::Command`/`Tick::Inbound`、Go HTTP/WS action 和 payload；
- **authority**：谁生成集合、计数、snapshot、排序、权限和 audience；
- **终态**：哪一个 `PersistOk` 后的 durable row/read-back 与唯一 `{event,data}` 才算成功；
- **失败**：非法输入、未知命令、HTTP/WS 错误、业务 `status != SUCCESS`、PersistErr、timeout、取消、错误 scope；
- **生命周期**：`corr_map`、pending、timer、WS、temporary id、重复/迟到回灌如何注册、匹配和释放；
- **兼容边界**：旧字段/旧 event 是否删除；如不保留，测试必须证明 fail-closed；
- **证据等级**：`FOCUSED`、`LOCAL_INTEGRATION`、`REAL_NATIVE_CHAIN`、`UI_DOM` 分别要拿什么。

字段合同按相邻边界登记，不写“最后应该有值”这种不可证伪目标：

| 边界 | 需要固定的内容 | 最低证据 |
|---|---|---|
| Go → Helix | action、完整 data、字段类型、audience、`corr`/`seq` | 同一轮 raw WS/HTTP |
| Helix 入站 → Persist | handler、解析结果、StorageOp、同一 channel/post | focused/host log |
| Persist → Projection | `PortOutcome::Ok`、读回行、snapshot/version | SQLite/read-back |
| Projection → Emit | 稳定 event、绝对字段、字段类型、Emit 次数 | EventSink receiver |
| driver/Tauri → Angular | `im:__bus__` 原文、批次、序列化形状 | bus/IPC log |
| Angular → UI | adapter 输出、upsert 结果、最终 view model/DOM | focused/UI test |

绝对投影字段必须标注语义：集合是 viewer 视角还是频道全局视角，计数是否等于集合长度，snapshot 是否单调，空集合如何表示。不要把 delta（例如单个 `readerUserId`）当成最终 UI 合同。

## 2. TDD 编码顺序

每一层先写会失败的最小测试，确认失败原因后再实现；禁止先改生产代码、最后补只断言 Emit 次数的测试。

### 红：按公开 seam 锁住缺口

1. **Go**（本轮授权 Go 时）：锁定 action、body、字段类型、广播 `userIds`/排除规则、远端持久事实和重复请求行为；不以 HTTP 200 代替业务终态。
2. **Helix**：从公开 `Tick`/handler seam 进入，断言首批 Effect、WS/PortReply 回灌、`Effect::PersistAtomic`、`PersistOk` 屏障、projection 字段和唯一 `Effect::Emit`。覆盖错误 status、PersistErr、timeout/取消、重复/迟到/旧 snapshot。
3. **driver/host**：断言 Effect 被正确执行，事件 envelope 原样保留，批量边界不拆业务 payload；不要在 driver 增加 reducer。
4. **Angular**：断言 adapter 只读取绝对 projection，缺字段时显式失败或保持未完成，不根据 `readerUserId`、旧列表或本地缓存做差集/刷新/兜底。

测试优先走公开 seam；纯确定性 projection helper 只有在没有更近公开入口时才直接测，并在报告标为 `FOCUSED`。每个异步 Effect 必须有对应的 PortReply/Inbound/Timer 回灌和清理断言。

### 绿：只改字段 owner

- Go 只修 wire、audience、远端 authority 或持久化；保持稳定字段名和类型，不把旧 delta 偷渡成新 projection。
- Helix 读取权威输入，在 matching `PersistOk` 后构造完整绝对 projection；固定字段复制保持有界 O(1)，不做无界 JSON read-modify-write。
- driver 只修 dispatch、序列化、receiver 生命周期和批量交付。
- Angular 只修 intent、envelope 解包、类型校验和 projection 映射；禁止 `??` 默认业务值、客户端集合运算和跨 event join。

如果 raw Go frame 已有值而 Angular 没有，按顺序检查 Helix 入站、SQLite/read-back、projection、EventSink、bus、adapter；只修第一处失配，不能直接改最后的 UI。

### 重构：保持同一事实来源

只有 focused 红绿通过后才整理重复代码。优先扩展现有共享 adapter、`messageV3-domain/shared` helper、公共 projection builder 或事件工厂；不要为每个 Gate 复制 HTTP、WS、SQLite、engine loop 或一套客户端 reducer。

## 3. Projection / Emit 专门闸门

每个 Gate 的具体 event 名称、字段、audience 和绝对/增量语义以当前 Go wire、Helix contract 与源码为准；以下是不变的实现顺序：

```text
Angular 提交最小 intent
  → IPC/Tauri command
  → Helix Tick/Effect
  → Go authority 处理对应 action
  → Go 定向 WS/HTTP 返回 authority observation
  → Helix 验证 scope/version 并 PersistAtomic
  → matching PersistOk/read-back
  → Helix Emit {event,data}
  → driver/Tauri 原样交付
  → Angular 只消费合同规定的绝对 projection 字段
```

通用修复合同：

1. raw frame/HTTP response 只是输入或 observation 证据，不等于最终 projection；单个 delta、局部成员或局部计数不能被客户端当成全量事实。
2. Go/Helix 必须明确 audience、viewer 视角、scope 和 version，并生成合同要求的完整绝对字段；不要让 Angular 计算差集、补计数或拼接跨 event 状态。
3. Helix 只有在 matching `PersistOk`、版本/快照和业务 scope 通过后才能 Emit；旧、重复或乱序输入必须幂等且不能倒退。
4. Angular 收到 projection 后直接替换对应视角的 read model；字段缺失、类型错误或 scope 不匹配必须暴露 `CONTRACT_ERROR`，不得回退旧列表、再次请求或默认业务值。
5. 测试必须覆盖 actor/observer 或 sender/receiver 视角（适用时）、空集合、多人集合、重复/乱序事件、断线恢复和错误终态；不适用的维度显式写 `N/A`。

## 4. 真实链验收

进行已授权的真实链验收时，触达外部 Go/WS/SQLite/Native 的 Command 使用共享 host/Native 装配，按八步留证。实现、定向验证、真实验收与发布分开报告；环境缺失不阻断独立的本地工作，但不得把实现完成冒称真实验收或整体任务完成：

1. 公开入口接收 payload，记录 `run_id`/`corr`；
2. Helix 产生首批 Effect，记录 optimistic/pending（如适用）；
3. host 执行真实 HTTP/WS/Storage/Emit，不走测试旁路；
4. 记录本轮 Go HTTP/WS raw 请求、响应、action、audience 和 seq；
5. 将 PortReply/WS inbound 回灌 Helix，确认状态机继续推进；
6. 等待 `PersistOk`，读取本轮 SQLite/远端 durable row 或投影状态；
7. 确认唯一终态 `{event,data}` 到达 EventSink，并核对字段 ledger；
8. 关闭 session、WS、engine 和本轮资源，验证无 pending/timer/corr/临时频道残留。

只有同一轮同时具备 HTTP/WS、Helix projection、持久化、EventSink 和适用的客户端证据，才可写 `REAL_NATIVE_CHAIN`。health 200、Accepted、静态 manifest、编译成功、单条 raw WS、单点 EventSink、截图或手工改库都不能晋级。

## 5. 验证与停止条件

按受影响仓库运行聚焦测试，避免无关全量扩张：

```bash
# Helix：先聚焦红绿；完整模块完成后再按就近 AGENTS 运行 Cargo
cargo test -p helix-im --lib <focused_filter> -- --nocapture
cargo test -p helix-driver-native --test message_v3_domain <gate> -- --nocapture --test-threads=1

# Go / Angular：使用各自仓库现有 package/test target，不臆造服务或凭据
go test ./<affected-package>/...
yarn test <affected-filter>

git diff --check
```

Rust 允许必要 focused 红绿和失败后的定向复验；禁止每处小编辑后重复宽范围检查，已通过结果按代码/依赖/环境身份复用，次数不作硬性停工上限。第二个无关失败出现时停止扩张、单列受阻门禁，继续独立批次；真实 Go/认证/WS/SQLite/Tauri 前置缺失时仅该验收层标 `BLOCKED_PRECONDITION`，不搭假服务冒充真实链。

## 6. 失败分类与交付合同

使用以下分类，不把“实现了代码”写成“链路已通过”：

- `CONTRACT_ERROR`：payload、event、字段类型、视角或终态合同与当前源不一致；
- `PROJECTION_LOSS`：authority/read-back 有值，Helix projection/event 丢值；
- `BRIDGE_LOSS`：EventSink 有值，driver/Tauri bus 丢值；
- `CLIENT_ADAPTER_LOSS`：bus 有值，Angular adapter/store 丢值；
- `REAL_CHAIN_FAILED`：真实边界已启动但请求、回灌、持久化或终态失败；
- `BLOCKED_PRECONDITION`：必要 Go/认证/WS/SQLite/Tauri/宿主不存在；
- `CLEANUP_FAILED`：本轮资源未释放；
- `UNVERIFIED`：只有静态或 focused 证据。

报告必须包含：

```text
结论：FIXED | PARTIAL | BLOCKED | UNVERIFIED
范围：仓库、branch/HEAD、文件租约和保留的 dirty/untracked
合同：入口、authority、expected event、字段绝对语义、失败终态
调用链：Angular → IPC → Helix → Go → Persist/Projection → Emit → Angular
第一失配：file:line + symbol + observed/expected
改动：只列各 owner 文件和协议/字段影响
FOCUSED：红测与绿测命令/结果
LOCAL_INTEGRATION：host、SQLite、transport 证据（若运行）
REAL_NATIVE_CHAIN：run_id/corr、HTTP/WS、Persist/read-back、EventSink、客户端证据
UI_DOM：viewport、状态和证据（若运行）
未验证/阻塞：缺失的真实前置和不能替代的证据
清理：run-owned 资源及结果
```

## 7. 提交边界与反模式

- 跨仓改动按仓库、owner 和可独立验收批次提交；使用中文 Conventional Commit 和 `docs/COMMIT_CONVENTION.md` 的结构化 body，不自动 push。
- 本批次实现未完成、适用门禁失败、已运行资源 cleanup 失败或暂存集合混入无关改动时禁止提交。无关工作树 dirty 不自动阻断；已满足本仓实现门禁的独立批次可按已有授权提交，真实验收/发布受阻单独报告。CI、合并、发布的真实链准入保持不变；不得将部分完成标记为整体完成。
- 不把 Go raw WS 直接当 Angular 事实，不把 driver 当 projection，不把 Angular fallback 当修复，不把 HTTP 200/health/mock/fixture/单库读回当真实链，不手工改共享数据库制造绿。
- 任何新函数/测试 helper 遵守就近 `AGENTS.md` 的紧邻精简注释、生命周期和热路径约束；固定字段复制必须有界，禁止重复 clone 和无界遍历。

## 参考真源

- [`messagev3-cross-layer-emit-diagnosis`](../messagev3-cross-layer-emit-diagnosis/SKILL.md)：第一丢失点、逐跳字段 ledger 和证据报告。
- [`references/source-map.md`](../messagev3-cross-layer-emit-diagnosis/references/source-map.md)：当前源码检索入口，不能替代实时定位。
- [`helix-command-driver-host-test`](../helix-command-driver-host-test/SKILL.md)：共享 adapter、G01a 八步和 Native/Go real-chain。
- [`docs/GOAL.md`](../../../docs/GOAL.md)、[`docs/ROADMAP.md`](../../../docs/ROADMAP.md)：架构不变量与当前活账。
- [`crates/helix-im/docs/projection-schema.md`](../../../crates/helix-im/docs/projection-schema.md)：MessageV3 projection/事件 schema。
