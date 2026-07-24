# Feature Specification: G01 WDIO 前五阶段验收

**Feature Branch**: `main`

**Created**: 2026-07-24

**Status**: Draft

**Loop Profile**: `helix-loop/v1`

**Input**: User description: "根据 G01 字段级设计和原生桌面测试体系，设计 WDIO 前五阶段的 G01 验收，覆盖 Mock、真实调用、单端、双账号、console 与 Tauri 日志、断言、权限、效率优化和第一版测试报告。"

## User Scenarios & Testing *(mandatory)*

### User Story 1 - 建立隔离且可证明的测试入口 (Priority: P1)

测试负责人可以在 Mac Intel 真实桌面应用上启动测试专用会话，并证明测试能力不会进入生产构建，也不会扩大生产权限。

**Why this priority**: 后续 UI、Mock、真实调用和双实例都依赖这一安全边界。仅安装依赖或进程启动不代表 G01 可测试。

**Independent Test**: 使用测试构建创建一个原生会话，读取当前窗口地址和测试能力状态；随后检查生产构建不存在相同入口。

**Acceptance Scenarios**:

1. **Given** Mac Intel 测试环境和测试构建，**When** 启动应用并创建自动化会话，**Then** 架构、状态端点、当前窗口和测试能力均可被确认。
2. **Given** 生产构建，**When** 扫描插件、capability 和测试桥接入口，**Then** 所有测试专用入口均不存在或不可调用。
3. **Given** 未授权的命令、路径或敏感字段请求，**When** 测试桥接收到请求，**Then** 请求被拒绝且不会把凭据或原始业务负载写入日志。

---

### User Story 2 - 单实例 G01 UI、Mock 与真实集成分层验收 (Priority: P1)

开发者可以在同一真实 WKWebView 中分别运行 UI、Frame V2 Mock、IPC reject 和真实 Tauri/Helix 集成测试，并从报告中看出每一层究竟证明了什么。

**Why this priority**: Mock 和 command receipt 很容易被误报为真实发送成功。分层验收是 G01 测试可信度的基础。

**Independent Test**: 单实例执行四组互不冒充的场景：纯 UI、Helix-derived Frame V2 Mock、IPC reject、真实 command/Frame/DOM。

**Acceptance Scenarios**:

1. **Given** message-v4 SurfaceRuntime ready，**When** 输入唯一 marker 并操作 Composer，**Then** 输入、提交、disabled/loading 与 DOM 合同可被自动断言。
2. **Given** `im_send_message` 返回 command receipt，且测试随后重放由真实 Helix 契约导出的 `sending -> sent` Frame V2，**When** 执行 Mock 场景，**Then** 调用参数、调用次数、Frame 解码、临时身份收敛和终态 DOM 均通过，但报告只标记 `mock-pass`。
3. **Given** 同一 command receipt 和 `sending -> failed` Frame V2，**When** 执行业务失败 Mock，**Then** DOM 失败态来自 Frame，而不是 Promise reject。
4. **Given** Tauri invoke 被一次性 reject，**When** 用户提交消息，**Then** 测试把它归类为 `ipc-rejected`，不冒充 Helix 业务失败。
5. **Given** Mock 已被清理，**When** 调用真实健康检查、只读查询和发送 command，**Then** Rust command 真正执行，console 与 Tauri/Helix 日志能够关联，报告标记 `integration-pass` 而非 canonical Green。
6. **Given** 任一单实例场景失败，**When** 测试结束，**Then** 自动保留截图、DOM 摘要、前端日志摘要、Tauri/Helix 日志摘要和断言错误。

---

### User Story 3 - 用 G01 替换慢速 raw WebDriver 代表链 (Priority: P1)

开发者可以使用标准化的元素操作、自动等待和事件关联执行 G01，而不再支付固定轮询、固定双实例、每步截图和成功路径全量证据扫描的成本。

**Why this priority**: 仅更换调用 API 而保留旧执行模型，无法解决十分钟级反馈问题。

**Independent Test**: 对同一个 G01 marker 分别运行迁移前代表链和迁移后代表链，比较字段、截图、DOM、correlation 和阶段耗时。

**Acceptance Scenarios**:

1. **Given** 同一 G01 场景输入，**When** 用新驱动执行 selector、click、type、attribute、execute、wait 和 screenshot，**Then** 业务断言与旧代表链等价。
2. **Given** 成功路径，**When** G01 达到终态，**Then** 只写紧凑 summary 和合同要求的终态证据，不执行失败级全量扫描。
3. **Given** 断言、Frame、日志或网络等待失败，**When** 进入失败升级，**Then** 自动补采截图、DOM、console、Tauri/Helix 和业务证据，首次失败不被 retry 覆盖。
4. **Given** 一个常驻单实例 suite，**When** 连续运行 Mock、真实调用和 G01 affected lane，**Then** 普通场景之间不重启 profile，不使用固定 sleep，也不启动 Observer。
5. **Given** 任一普通 Gate 超过 15 秒，**When** 报告完成，**Then** 报告列出 setup、action、command receipt、Frame、DOM、evidence、screenshot 和 report 各阶段耗时及等待原因。

---

### User Story 4 - 双账号 G01 真实发送与接收 (Priority: P1)

测试负责人可以在同一 run 中启动 Actor 444 和 Observer 678 两个隔离实例，由 Actor 发送 G01 文本，由 Observer 接收并渲染同一消息。

**Why this priority**: 只有双账号链才能证明真实投递、观察者消费和跨实例证据关联；单端 sent 不能替代 Observer 到达。

**Independent Test**: 使用唯一 run id 和 marker，Actor 完成发送，Observer 在真实链上收到相同 post，并生成双端终态证据。

**Acceptance Scenarios**:

1. **Given** Actor 444 与 Observer 678 使用不同 profile、认证、device id、端口和产物目录，**When** 两实例同时启动，**Then** 任一实例重启或失败都不会关闭或污染另一实例。
2. **Given** 双方已进入同一频道，**When** Actor 提交唯一 marker，**Then** Actor 先看到权威 Frame 驱动的发送态和终态，Observer 最终看到相同服务端消息身份。
3. **Given** 消息完成，**When** 汇总证据，**Then** command `reqId`、HTTP `trackId`、channel、temporary ID、server message ID、event sequence、Frame transaction 和双方 DOM 至少通过一组无歧义关联键闭合。
4. **Given** 双端 canonical 场景，**When** reducer 判定结果，**Then** Go、WS、本地 SQLite、远端 PG、Helix Frame、FrameApplied、Actor DOM、Observer DOM 和双端截图缺一即不是 Green。
5. **Given** Observer 未收到或证据关联冲突，**When** 场景超时，**Then** 结果为 failed/blocked，报告不得用 Actor sent 或 HTTP success 代替跨账号完成。

---

### User Story 5 - 产出第一版可读且可机器解析的测试报告 (Priority: P2)

开发者和 CI 可以从同一 run 读取快速本地结果、标准化 case 结果、阶段耗时、双端附件索引和业务证据判定。

**Why this priority**: 没有统一报告，Mock、integration 和 canonical 结果会再次混在一起，性能改进也无法量化。

**Independent Test**: 人为制造一个 pass 和一个 fail，确认本地摘要、标准 XML、run manifest、日志和截图索引一致，且退出码与失败数一致。

**Acceptance Scenarios**:

1. **Given** 一次 G01 run，**When** 执行结束，**Then** 生成终端可读结果、JUnit XML、紧凑 summary、run manifest 和 evidence 引用。
2. **Given** Mock、integration、single-real-chain 和 dual-real-chain 四种层级，**When** 查看报告，**Then** 每个 case 显示自己的层级、证明范围和不可声称事项。
3. **Given** 测试失败，**When** 查看报告附件，**Then** 可按 run id、Gate、actor 和失败阶段定位 screenshot、console、Tauri/Helix 日志与 evidence summary。
4. **Given** 报告生成，**When** 执行脱敏检查，**Then** token、authorization、Cookie、账号密码、原始消息正文和 raw payload 均不存在。
5. **Given** 测试发生 retry，**When** 最终 retry 通过，**Then** 首次失败仍被保留并标记 flaky，不得覆盖原始附件。

## Loop Contract *(mandatory)*

### Trigger

从测试负责人选择 G01 的 `mock`、`invoke`、`single-real` 或 `dual-real` lane 开始；每次运行必须先生成唯一 run id、Gate id、lane 和 persona 上下文。

### Goal

在 Mac Intel 真实 Tauri WKWebView 上，以分层且可证伪的方式完成 G01 发送消息测试：单实例覆盖 UI、Helix-derived V2 Mock 和真实 Tauri/Helix；双实例覆盖 444 发送、678 接收；所有层均有断言、console/Tauri 日志和第一版报告。

### Completion

完成不是 command receipt、Mock resolve、HTTP success 或 Actor DOM 单面通过。Feature 完成需要：

- 五个阶段各自达到本 Spec 的验收条件；
- Mock、integration、single-real-chain、dual-real-chain 分开判定；
- 双账号 G01 的必需 Go/WS/SQLite/PG/Frame/FrameApplied/DOM/截图证据闭合；
- 第一版报告与退出码一致；
- 性能预算达标且没有通过删除必要证据换取速度；
- 生产构建不含测试插件、capability 或桥接入口。

### Boundaries

- 本 Feature 只验收 G01 文本发送，不包含 G02 文件上传、G04-G16 或 GS01-GS04 的业务 Green。
- 第五阶段只证明 G01 已可接入现有恢复 Harness；不会因 stop/restart 控制可用而宣称 GS Gate 完成。
- Allure、视觉基线服务和 Mac CI 发布流水线不阻塞第一版报告。
- 同一失败签名最多自动修复两次；第三次进入 `NeedsHuman`。
- 普通 DOM 等待 5 秒、command/Frame 10 秒、真实投递 15 秒；双端总场景预算 30 秒。更长等待必须有独立阶段预算和报告。

### Human Gate

- 首次授予 macOS 屏幕录制或自动化控制权限需要人工确认。
- 首次建立或更新视觉基线需要人工审核；本 Feature 不自动批准像素基线。
- 测试账号、PG 只读凭据或远端环境不可用时必须由用户恢复，不允许自动绕过认证或扩大权限。

### Edge Cases

- command receipt 成功但没有任何 Frame。
- Mock Frame 与当前 Helix schema/hash 不一致。
- `sending` Frame 到达但 terminal Frame 未到达。
- HTTP `SUCCESS` 没有 server ID，身份随后由 WS/recovery 补齐。
- HTTP/IPC 失败、15 秒超时和迟到 WS echo 发生不同顺序。
- Actor sent 但 Observer 未收到。
- 双实例端口、profile、auth、device id 或 artifact 路径冲突。
- console/Tauri 日志丢失、乱序、跨 run 混入或包含敏感字段。
- Frame revision/epoch gap 导致 fail-closed 和 snapshot request。
- retry 最终通过但首次失败证据被覆盖。
- WebDriver 会话断开后只恢复受损 persona，不重启完整拓扑。

## Requirements *(mandatory)*

### Functional Requirements

#### 阶段一：测试入口、安全与权限

- **FR-001**: 系统 MUST 在测试专用构建中提供原生 UI 自动化、Tauri execute/Mock 和前后端日志能力。
- **FR-002**: 系统 MUST 证明生产构建不包含测试插件、测试 capability、Mock 桥接或调试日志入口。
- **FR-003**: 测试权限 MUST 遵循最小授权，只允许已登记的窗口、命令、loopback 端口、profile 和 artifact 路径。
- **FR-004**: macOS 屏幕录制/截图、进程控制、端口绑定和文件写入权限 MUST 在运行前显式 preflight；缺失时 fail fast 并指出具体权限。
- **FR-005**: Actor/Observer 凭据 MUST 从受控环境或隔离 profile 注入，报告和日志不得保存密码、token、authorization 或 Cookie。
- **FR-006**: SQLite 与远端 PG 只用于只读验收查询；测试不得用直接数据库写入修复或制造 Green。

#### 阶段二：单实例 UI、Mock 和真实调用

- **FR-007**: 系统 MUST 等待 message-v4 SurfaceRuntime ready 后再操作 Composer，禁止固定 sleep 作为正常控制流。
- **FR-008**: G01 Mock MUST 同时包含 command receipt 和由当前 Helix 契约代码导出或真实链捕获的完整 Frame V2 fixture。
- **FR-009**: 每个 Frame fixture MUST 记录 Helix commit、producer、schema、语义 hash 和捕获/导出方式；测试代码不得自行补造缺失 Surface 或领域字段。
- **FR-010**: Mock MUST 覆盖 `sending -> sent`、`sending -> failed`、调用参数、调用次数、once/reset/restore 和未知 command。
- **FR-011**: IPC reject MUST 与 Frame 驱动的业务失败分开测试和报告。
- **FR-012**: 真实 integration MUST 在清除 Mock 后调用真实健康检查、只读 query 和发送 command，并证明 Tauri/Rust/Helix 路径真实运行。
- **FR-013**: command receipt 只允许标记 command accepted；`sent/failed` DOM 必须来自 matching-causation Frame。
- **FR-014**: 单实例失败 MUST 自动保存截图、可见窗口 DOM 摘要、console 摘要、Tauri/Helix 摘要和断言错误。

#### 阶段三：G01 驱动迁移与效率

- **FR-015**: G01 的 selector、click、type、text、attribute、execute、screenshot、session 和 wait MUST 使用统一驱动能力，不再直接调用 raw W3C 客户端。
- **FR-016**: 旧 raw driver 只有在 G01 代表链的产物字段等价、无调用残留并完成回归后才允许删除。
- **FR-017**: Mock/UI、真实单实例、双账号/恢复 MUST 使用按需 lane；单账号 lane 不得启动 Observer。
- **FR-018**: suite MUST 复用常驻拓扑；除明确的重启场景外，普通 case 之间不得重启 profile。
- **FR-019**: DOM、Frame、console、Tauri/Helix、Go 和 Storage 等待 MUST 优先采用事件 latch 或按 correlation 的增量索引，不得循环扫描全部日志和 DOM。
- **FR-020**: 成功路径 MUST 只采合同要求的紧凑证据；失败路径 MUST 自动升级为完整诊断采集。
- **FR-021**: 每个 case MUST 记录 `setupMs/actionMs/commandAckMs/frameMs/domMs/evidenceMs/screenshotMs/reportMs/totalMs`。

#### 阶段四：双实例双账号 G01

- **FR-022**: 系统 MUST 支持 Actor 444 与 Observer 678 同 run 并发，且 profile、auth、device id、端口、日志和截图目录完全隔离。
- **FR-023**: Actor MUST 通过真实 UI 提交唯一 marker；Observer MUST 通过自己的 Helix Frame 和 DOM 收到服务端消息，不得注入测试 Frame。
- **FR-024**: 双端 G01 MUST 关联 command、Go HTTP、PG commit、WS/recovery、SQLite、Frame、FrameApplied 和双方 DOM。
- **FR-025**: 双端 G01 Green MUST 要求 Actor 与 Observer 的终态截图属于同一 fresh run；历史截图或其他 run 证据不得复用。
- **FR-026**: 任一 persona 失败时，系统 MUST 保留另一 persona 的现状，并只恢复损坏实例。
- **FR-027**: 双实例日志 MUST 含 run id、Gate、persona、process start marker、时间范围和允许的 correlation 字段，以阻止旧进程或跨 run 日志混入。

#### 阶段五：G01 canonical 接入与第一版报告

- **FR-028**: 第五阶段 MUST 用 G01 证明现有 Native Harness 的 stop/restart/profile 控制可以与统一驱动共存，但不得据此声明 GS01-GS04 Green。
- **FR-029**: 第一版报告 MUST 同时提供终端可读结果、JUnit XML、紧凑 JSON/Markdown summary、run manifest 和 evidence 路径索引。
- **FR-030**: 报告 MUST 为每个 case 标记 `ui-only/mock/integration/single-real-chain/dual-real-chain` 之一，并列出该层不可声称的证据。
- **FR-031**: 报告 MUST 区分 build、startup、action、invoke、Frame、DOM、evidence、screenshot 和 report 失败阶段。
- **FR-032**: 报告 MUST 记录 pass/fail/skip、开始/结束时间、耗时、重试次数、首次失败和最终退出码。
- **FR-033**: Reporter 附件 MUST 使用 allowlist 脱敏；消息正文只允许以不可逆 marker hash 或受控测试 marker 表示。
- **FR-034**: 第一版报告 MUST 支持从 failed case 定位 Actor/Observer 截图、console、Tauri/Helix 日志和 evidence summary。
- **FR-035**: Allure 和视觉基线服务 MAY 后续增加，但不得成为本 Feature 完成依赖。

### G01 必须断言

| 层级 | 必须断言 | 明确不能声称 |
|---|---|---|
| UI-only | Surface ready、Composer 输入/提交、selector/CSS/尺寸、失败截图 | command、Helix、Go、持久化 |
| Mock | command payload/calls、V2 decode、`sending -> sent|failed`、DOM identity/delivery | Rust command、Go、SQLite/PG |
| Integration | Mock 已清除、真实 invoke、AcceptedCommand、console/Tauri/Helix、matching Frame、DOM | Observer 到达；缺 Go/PG 时不称 real-chain |
| Single real-chain | Actor UI、Go HTTP、SQLite、PG、Frame、FrameApplied、Actor DOM | Observer 消费 |
| Dual real-chain | Actor/Observer 全链 correlation、双方 DOM/截图、无重复 | GS01-GS04 |

### 日志字段与脱敏合同

| 日志面 | 允许字段 | 禁止字段 |
|---|---|---|
| Browser console | runId、Gate、persona、command、surface、scope kind、reqId、traceId、transactionId、status、count、duration | token、Cookie、authorization、账号密码、任意原始 body |
| Tauri/Helix | process start、pid、commit/pin、reqId、corr、effect kind、persist result、surface、revision、eventSeq、duration | auth header、完整 payload、消息正文 |
| Go/WS evidence | trackId、channelId、temporaryId hash、postId、eventSeq、event、commit/outbox status | 用户凭据、Cookie、原始消息正文 |
| Reporter | case/lane/result/timing、脱敏 correlation、artifact 路径 | 上述所有敏感字段和未裁剪日志文件 |

### 权限与运行前置

| 能力 | 测试侧所需权限 | 生产侧要求 |
|---|---|---|
| 原生 UI 会话 | 测试 build 的 WebDriver capability、独立 loopback 端口 | 不存在 |
| Tauri execute/Mock/log | 测试 allowlist command 与日志通道 | 不存在 |
| 截图 | WebDriver screenshot；系统截图回退需 macOS Screen Recording | 不自动申请 |
| 双实例 | 启停指定测试进程、隔离 profile/auth/device/port | 不影响正常 profile |
| 凭据 | 读取受控测试 profile 或安全环境变量 | 不写入 artifact |
| SQLite | 对本次 profile 数据库只读查询/快照 | 禁止直接修复 |
| PG | 只读证据账号和网络可达性 | 禁止 schema/data 管理权限 |
| Go/WS | 已认证的测试环境连接 | 不绕过服务端鉴权 |
| Artifact | 仅写当前 run 目录 | 不访问用户其他目录 |

### Key Entities

- **G01Run**: 一次隔离运行；包含 run id、Gate、lane、candidate、开始结束时间和结果。
- **TestPersona**: Actor 444 或 Observer 678；拥有独立 profile、auth、device、端口、进程和 artifact namespace。
- **CommandReceipt**: 命令接管回执；包含 request correlation，但不表示业务完成。
- **FrameFixtureProvenance**: Mock Frame 的 Helix commit、producer、schema、hash 和来源。
- **G01MessageIdentity**: temporary ID、server message ID、channel、event sequence 和 marker hash 的映射。
- **EvidenceBundle**: console、Tauri/Helix、Go/WS、SQLite、PG、Frame、FrameApplied、DOM 和 screenshot 的关联索引。
- **AssertionResult**: assertion id、evidence plane、期望、实际、耗时和失败阶段。
- **G01ReportV1**: Spec/JUnit/summary/manifest 的组合，引用但不复制完整敏感日志。
- **PerformanceBreakdown**: build、startup、setup、action、receipt、Frame、DOM、evidence、screenshot 和 report 的阶段耗时。

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: 五个阶段的 mandatory case 全部有明确 pass/fail/blocked 结果，0 个 case 用 Mock 或中间回执冒充更高层 Green。
- **SC-002**: 单个 Mock/UI case 的 warm 反馈时间 P95 不超过 10 秒。
- **SC-003**: 单实例 G01 warm real-chain 的场景执行 P95 不超过 15 秒，且普通场景 profile restart 次数为 0。
- **SC-004**: 双实例 G01 warm real-chain 的场景执行 P95 不超过 30 秒，并且一次 suite 只启动一套所需拓扑。
- **SC-005**: G01 成功路径不执行固定 before/final 双端全量截图；失败或 canonical 合同要求时截图覆盖率为 100%。
- **SC-006**: Actor/Observer canonical run 的九个证据面关联完整率为 100%，任何缺面都被报告为非 Green。
- **SC-007**: 第一版 JUnit testcase 数、失败数、跳过数和进程退出码一致率为 100%。
- **SC-008**: 每个 failed case 都能在 30 秒内从报告定位到失败阶段、persona、截图、console、Tauri/Helix 日志和 evidence summary。
- **SC-009**: 报告和附件的敏感字段扫描结果为 0 条凭据、token、authorization、Cookie 或账号密码泄漏。
- **SC-010**: 生产构建的测试插件、capability、Mock bridge 和测试日志入口静态/运行态检出数均为 0。
- **SC-011**: raw G01 代表链与新链的必需业务字段和 evidence plane 等价率为 100%，新链不存在 raw client 直接调用。
- **SC-012**: 任一超过 15 秒的普通 case 都有完整阶段耗时和等待原因，无法归因的慢 case 数为 0。

## Assumptions

- Mac Intel 主机、真实 WKWebView、系统截图权限和现有 Native Harness 可用。
- Actor 444 与 Observer 678 的认证由现有隔离 profile 或安全注入提供；Spec 不保存密码。
- 当前 Helix 与 cses-client 只接受 InteractionFrame V2；G01 CURRENT producer 仍以 `message.timeline` 为主要发送 Surface。
- `PendingSend` 是 Helix 内存聚合，本地 durable 发送事实在 SQLite `message` 表。
- G01 Mock fixture 由当前 Helix producer/serializer 导出或从真实链捕获，测试仅参数化 correlation、身份和 marker。
- 第一版报告采用轻量本地摘要和标准 XML；富报告与视觉差异留到后续 Feature。
- 真实链依赖可达的 Go IM 环境、WS、只读 PG 证据入口和本地 SQLite。
- Phase 5 只建立 G01 对恢复 Harness 的接入能力，不扩大本 Feature 到 GS01-GS04。
