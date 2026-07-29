# Implementation Plan: G01 WDIO 前五阶段验收

**Branch**: `019-g01-wdio-five-phases` | **Date**: 2026-07-24 | **Spec**: [spec.md](spec.md)

**Input**: G01 字段级设计基线、现有 Native raw W3C Harness、真实 Tauri/WKWebView 测试路线，以及用户确认的“测试安全入口 → 单实例 Mock/real → G01 驱动迁移 → 444/678 双端 → Report V1”顺序。

## Summary

在不改变 G01 生产业务语义、Go API 或数据库 schema 的前提下，为 cses-client 建立测试专用 WDIO 执行面。Helix 继续唯一生产领域事实：Mock 只模拟 `AcceptedCommand`，业务状态通过 Helix Rust 导出或真实链捕获并经合同校验的 InteractionFrame V2 fixture 从真实 `im:__bus__` 入口进入 Angular。真实 lane 必须清空 Mock 并分别证明真实 invoke、Actor 单端链和 444/678 双端链。执行器复用现有 Native topology/evidence 能力，以事件 latch、按 correlation 增量索引和失败升级采集替换固定 sleep、反复重启、全量日志扫描；Report V1 同时输出终端摘要、JUnit、JSON/Markdown summary、manifest 和脱敏附件索引。

## Technical Context

**Language/Version**: TypeScript 5.6、Angular 20.3、Node.js 20；Tauri 2.10/Rust（cses-client 锁定工具链）；Helix Rust workspace 当前工具链

**Primary Dependencies**: WebdriverIO/Mocha/expect-webdriverio、Tauri WDIO 测试插件、cses-client `SurfaceRuntime`/`ShellService`、现有 `tests/e2e/desktop` topology/evidence、Helix `InteractionFrameV2`

**Storage**: 测试产物文件系统；Actor/Observer 本地 SQLite 与远端 PostgreSQL 仅做有界只读证据查询

**Testing**: WDIO Mocha specs、Node/Jest contract tests、Helix Rust fixture contract tests、Tauri test-build/production-boundary checks、现有 Native evidence reducer

**Target Platform**: Mac Intel x86_64 macOS 的真实 Tauri WKWebView；后续 CI 可复用合同但不在本 Feature 内交付

**Project Type**: 双仓测试纵切：Helix 提供 V2 fixture 真源；cses-client 提供 Tauri/Angular/WDIO runner、双实例拓扑与报告

**Performance Goals**: warm Mock/UI P95 ≤10s；单实例 real-chain P95 ≤15s；双实例 real-chain P95 ≤30s；correlation lookup 平均 `O(1)`，禁止每次等待重扫完整日志

**Constraints**: 测试插件/capability/bridge 不进入生产构建；无固定 sleep；成功路径只采紧凑证据；失败保留首次失败并升级采集；凭据与正文不入日志；普通单端 lane 不启动 Observer

**Scale/Scope**: G01 一个 Gate、五阶段、两 persona、五个证据等级、九个 canonical evidence plane；不宣称 G02/G04-G16 或 GS01-GS04 Green

## Constitution Check

*GATE: Phase 0 前及 Phase 1 后均必须通过。*

| 原则 | 设计判定 |
|---|---|
| Runtime-Agnostic Layering | PASS：Helix 只新增 fixture contract/export 测试，不依赖 WDIO/Tauri/UI；runner 与平台权限留在 cses-client |
| Deterministic Facts | PASS：Mock Frame 来自 Helix serializer/producer 或真实捕获，WDIO 不补造 Surface/领域字段 |
| V2-only Ownership | PASS：fixture 只接受 schemaVersion=2；Angular 仍通过 `im:__bus__` 与 SurfaceRuntime apply |
| Evidence Before Completion | PASS：receipt、Frame、SQLite、PG、Go/WS、FrameApplied、DOM 和截图分层判定 |
| Security By Construction | PASS：WDIO 插件、capability 与 bridge 均受 test-only feature/config 控制，生产 build 有负向检查 |
| Minimal Change | PASS：复用 `tests/e2e/desktop` topology/evidence，不重写登录、进程控制或数据库 collector |
| Harness Admission | PASS：R1 已通过；Plan 接受 R2，Tasks 接受 R3 后才允许实现 |
| 热路径与复杂度 | PASS：run-scoped map/index + event latch；不引入业务热路径或数据库写入 |

**Post-design re-check**: PASS。`data-model.md` 只有测试运行实体；`contracts/` 不改变生产协议；`quickstart.md` 的数据库步骤均为只读，生产构建负向检查是第一道 gate。

## Loop Detail Contract

### Inputs and Context

- **I-001**: Helix 当前 G01 V2 producer、`crates/helix-im/tests/interaction_frame_contract_test.rs`、fixture manifest 与 G01 字段级设计。
- **I-002**: cses-client 当前 `feat/helix-reactor-im` source pin、`src-tauri/Cargo.toml` 的 `desktop-e2e` feature、`framework/plugins.rs` 和 Angular `ShellService`/SurfaceRuntime。
- **I-003**: cses-client `tests/e2e/desktop` 的 raw W3C driver、NativeTopology、EvidenceCollector、G01 scenario 与双 profile runner。
- **I-004**: [research.md](research.md) 的 package、安全边界、拓扑、fixture、等待、报告和迁移决策。
- **I-005**: 444/678 隔离 profile、macOS 权限、已认证真实环境、SQLite/PG 只读能力；缺失时 fail fast。
- **I-006**: [contracts/g01-wdio-acceptance.md](contracts/g01-wdio-acceptance.md)、两个 JSON Schema 与日志脱敏合同。

### Action Chain

- **A-001**: 锁定双仓 source/lock/fixture hash → 增加 test-only WDIO feature/capability/config → 负向证明 production build 无测试入口。
- **A-002**: Helix Rust 产生 G01 `sending/sent/failed` V2 fixture 与 provenance → client 校验 hash/schema → Mock receipt 与 Frame 分离重放。
- **A-003**: 单实例常驻 suite 执行 UI-only、Mock、IPC reject、真实 invoke、single-real；通过 run-scoped event latch 等待 Frame/DOM/log。
- **A-004**: 为 G01 建立统一 WDIO adapter/page object，证明与旧 raw 代表链字段等价后切断 G01 的 raw 调用；其他 Gate 尚未迁移时保留共享旧 driver。
- **A-005**: 仅 dual lane 启动 444/678；Actor 真实 UI send，Observer 真实 WS/Helix/DOM receive；collector 关联九个证据面。
- **A-006**: reporter 聚合 Spec/Mocha/JUnit/summary/manifest/附件索引，执行 allowlist 脱敏与首次失败保留，输出退出码。
- **A-007**: 任一等待超时或证据冲突 → 标记 failed/blocked → 只恢复损坏 persona → 采集完整诊断，不用 receipt/Actor sent 降级冒充。

### State Contract

- **S-001**: `Unconfigured` → `SafeHarnessReady`：测试会话可建且 production-boundary 检查通过。
- **S-002**: `SafeHarnessReady` → `SingleLayeredReady`：UI/Mock/IPC/real invoke 各自通过且证据等级无越级。
- **S-003**: `SingleLayeredReady` → `G01WdioMigrated`：G01 等价合同通过、G01 raw 调用为 0、warm 时延满足预算。
- **S-004**: `G01WdioMigrated` → `DualRealPassed`：444/678 同 run 九面证据闭合且双端截图新鲜。
- **S-005**: `DualRealPassed` → `ReportV1Closed`：报告合同、脱敏、retry/first-failure 与退出码一致。
- **S-006**: 任一状态 → `Blocked`：权限、认证、PG 只读、真实环境或 source/fixture pin 缺失。
- **S-007**: 任一可执行状态 → `NeedsHuman`：同失败签名第三次出现、首次授权或视觉基线审批。

### Validators

- **V-001**: Helix focused Rust fixture contract 证明 G01 fixture 可由当前 `InteractionFrameV2` 解码、producer/source pin/hash 匹配且三种终态齐全。
- **V-002**: cses-client Node/Jest contract tests 证明 JSON Schema、fixture mirror、脱敏、correlation index、timing 和 reporter 结果确定。
- **V-003**: test-build session/preflight 与 production-boundary scan 证明测试能力只在专用 feature/config 下可达。
- **V-004**: WDIO single suite 证明 UI-only、Mock `sending→sent|failed`、IPC reject、real invoke 和 single-real 分层通过。
- **V-005**: G01 equivalence test 证明 selector/action/DOM/correlation/artifact 字段不低于旧代表链，且 G01 raw call count=0。
- **V-006**: WDIO dual suite 证明 Actor/Observer 隔离、真实 send/receive、九面证据与双方 fresh screenshot。
- **V-007**: Report V1 injected pass/fail/retry contract 证明 Spec/JUnit/summary/manifest/附件、脱敏和退出码一致。
- **V-008**: 性能 reducer 证明 warm Mock/UI、single-real、dual-real P95 与零普通 profile restart 预算。

## Architecture And Dependency Direction

```text
Helix producer/serializer
  -> canonical G01 Frame V2 fixtures + provenance/hash
  -> client fixture mirror verifier

WDIO spec -> G01Page/DriverPort -> WDIO/Tauri test plugins -> real WKWebView
          -> invoke Mock receipt --------\
          -> im:__bus__ V2 replay --------> Angular SurfaceRuntime -> DOM
          -> real Tauri invoke -> Helix -> SQLite/Go HTTP/WS/PG -> V2 -> DOM

NativeTopology -> Actor/Observer process/profile isolation
EvidenceCollector -> run-scoped CorrelationIndex -> ReportV1
```

- Domain：Helix 拥有 message identity/delivery/Surface 事实，不依赖测试框架。
- Application：runner 编排 lane、persona、latch、恢复与证据等级，不解释业务 JSON。
- Infrastructure：WDIO/Tauri 插件、进程、文件、SQLite/PG reader 实现端口。
- Interface：page object、fixture bus adapter、reporter 只转换协议并执行断言。

## Delivery Phases

### Phase 0 - Source Lock And Research

记录 Helix/cses-client revision、client Helix pin、lockfile hash、现有 raw G01 调用与 fixture manifest。完成 `research.md`、`data-model.md`、`contracts/` 和 `quickstart.md`。

### Phase 1 - Test-only Entry And Safety

1. 锁定兼容的 WDIO/Mocha/Tauri 插件版本到 client lockfile。
2. 增加 additive test-only Cargo feature、capability/config 与最小 command allowlist。
3. 将插件注册限制在测试 build + explicit profile 双条件。
4. 增加 session/preflight 与 production-boundary negative test。
5. 初始化 run directory、persona namespace 和日志 allowlist。

**Exit**: V-002、V-003 通过；生产构建无法发现或调用测试能力。

### Phase 2 - Single-instance UI, Mock And Real

1. Helix 导出并 contract-test G01 `sending/sent/failed` V2 fixtures 与 provenance。
2. client mirror 校验 source pin、schema 与 semantic hash，禁止测试手写字段。
3. Mock command 仅返回 `AcceptedCommand`；Frame 从真实 `im:__bus__` 入口投递。
4. 独立验证 `sending→sent`、`sending→failed`、IPC reject、unknown command 与 reset/restore。
5. 清空 Mock 后执行真实 health/query/send，关联 console、Tauri/Helix、Frame 和 DOM。

**Exit**: V-001、V-004 single 部分通过；结果分别为 `ui-only/mock/integration/single-real-chain`。

### Phase 3 - G01 Driver Migration And Efficiency

1. 抽取 `G01Page` 与统一 driver port，覆盖 selector/click/type/text/attribute/execute/screenshot/wait。
2. Native topology 对接 WDIO session；常驻单实例 suite 不在普通 case 间重启 profile。
3. 用事件 latch 与 `Map<correlationKey, Event[]>` 增量索引等待 receipt/Frame/DOM/log。
4. 对比旧 raw 代表链的断言和 artifact 字段，等价后让 G01 不再直接调用 raw W3C client。
5. 其他 Gate 仍依赖共享旧 driver 时不物理删除 driver；只冻结其 G01 新调用。

**Exit**: V-005、V-008 single 部分通过；G01 raw call=0，普通 lane profile restart=0。

### Phase 4 - Dual-account Real G01

1. dual lane 按需启动 444 Actor 与 678 Observer，隔离 profile/auth/device/port/log/artifact。
2. Actor 真实 UI 输入 unique marker 并提交；Observer 不注入 Frame，只等待真实 WS/Helix/DOM。
3. 将 reqId/trackId/channel/temporaryIdHash/postId/eventSeq/transactionId 与双方 DOM 关联。
4. SQLite/PG 只读查询、Go/WS 摘要、FrameApplied 与 fresh screenshots 全部登记。
5. persona 失败时仅恢复该 persona，另一实例保持现场。

**Exit**: V-006 通过；九面缺一即 failed/blocked，不用 Actor sent 或 HTTP success 代替 Green。

### Phase 5 - Canonical Harness Coexistence And Report V1

1. 证明统一 driver 可与现有 stop/restart/profile 控制共存，但只报告 G01。
2. 输出 Spec/Mocha、JUnit XML、JSON/Markdown summary、manifest 和附件索引。
3. case 固定 evidence level、claim boundary、stage timing、retry 和 first failure。
4. 注入 pass/fail/retry 验证退出码、flaky 标记、附件路径和 allowlist 脱敏。
5. 聚合 warm 性能，不将 build/install/首次授权混入场景 P95。

**Exit**: V-007、V-008 全部通过；不宣称 GS01-GS04 Green。

## Project Structure

### Documentation

```text
specs/019-g01-wdio-five-phases/
├── spec.md
├── discovery.md
├── research.md
├── plan.md
├── data-model.md
├── quickstart.md
├── contracts/
│   ├── g01-wdio-acceptance.md
│   ├── g01-frame-fixture-provenance.schema.json
│   ├── g01-report-v1.schema.json
│   └── log-redaction-contract.md
├── verification.md
├── review-ledger.json
└── tasks.md
```

### Source Code

```text
crates/helix-im/
├── tests/g01_wdio_fixture_contract_test.rs
└── tests/fixtures/g01-wdio/

/System/Volumes/Data/workspace/angular/cses-client/
├── src-tauri/
│   ├── Cargo.toml
│   ├── capabilities/wdio-e2e.json
│   ├── tauri.wdio.conf.json
│   └── src/framework/plugins.rs
├── wdio.conf.ts
├── tests/e2e/wdio/
│   ├── config/
│   ├── fixtures/g01/
│   ├── harness/
│   ├── pageobjects/
│   ├── reporters/
│   ├── specs/
│   └── contracts/
├── tests/e2e/desktop/              # 复用 topology/evidence；迁移期保留 raw driver
└── scripts/message-real-chain/     # 复用真实证据 collector/reducer
```

**Structure Decision**: 不新增生产 runtime 或独立服务。Helix 只增加测试 fixture 真源；cses-client 在 `tests/e2e/wdio` 建立新执行面，通过 adapter 复用现有 desktop topology/evidence。旧 raw driver 只有在全部调用者迁移后才能删除，本 Feature 只要求 G01 路径归零。

## Complexity Tracking

无 Constitution 例外。correlation index 的写入与单键读取平均 `O(1)`、空间 `O(e)`；DOM 操作由 WDIO 显式等待，失败才采全量摘要。双实例只在 dual/recovery lane 启动，避免将固定拓扑成本施加到 Mock/UI。
