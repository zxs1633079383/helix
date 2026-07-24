# Tasks: G01 WDIO 前五阶段验收

**Input**: [spec.md](spec.md)、[plan.md](plan.md)、[research.md](research.md)、[data-model.md](data-model.md)、[contracts/](contracts/)

**Organization**: 任务按五个用户故事/阶段组织。所有测试任务先于实现；Helix 只拥有 Frame V2 fixture 真源，cses-client 拥有 WDIO/Tauri/Angular runner。G01 raw 调用归零不等于可删除其他 Gate 仍使用的共享旧 driver。

## Phase 1: Setup And Source Lock

- [ ] T001 记录 Helix/cses-client branch、HEAD、dirty status、client Helix pin、package/Cargo lock hash、G01 fixture manifest hash 和 raw G01 调用 inventory 到 `specs/019-g01-wdio-five-phases/verification.md`
- [ ] T002 [P] 在 `/System/Volumes/Data/workspace/angular/cses-client/package.json` 与 lockfile 锁定 Node 20 兼容的 WebdriverIO/Mocha/expect/reporter 依赖并增加 `message:wdio:*` 占位入口
- [ ] T003 [P] 在 `/System/Volumes/Data/workspace/angular/cses-client/src-tauri/Cargo.toml` 增加 additive `wdio-e2e` feature 与测试专用插件 optional dependencies，保持 default feature 不变
- [ ] T004 [P] 在 `/System/Volumes/Data/workspace/angular/cses-client/src-tauri/capabilities/wdio-e2e.json` 和 `src-tauri/tauri.wdio.conf.json` 定义最小测试 capability、loopback 端口与 artifact allowlist
- [ ] T005 [P] 在 `/System/Volumes/Data/workspace/angular/cses-client/wdio.conf.ts` 与 `tests/e2e/wdio/config/lanes.ts` 建立 `ui/mock/integration/single-real/dual-real/report` suite 配置和预算
- [ ] T006 在 `/System/Volumes/Data/workspace/angular/cses-client/tests/e2e/wdio/tsconfig.json`、`config/personas.ts` 与目录入口建立 Actor/Observer、fixture、harness、pageobjects、reporters、specs 和 contracts 结构

## Phase 2: Foundational Contracts And Harness

**Checkpoint**: T007-T020 完成前，任何 story spec 只能保持 RED，不能报告 lane Green。

- [ ] T007 [P] 在 `/System/Volumes/Data/workspace/angular/cses-client/tests/e2e/wdio/contracts/__tests__/json-schema.test.ts` 为 fixture provenance 与 Report V1 schema 增加 valid/unknown-field/missing-field/unsafe-path RED contract tests
- [ ] T008 [P] 在 `/System/Volumes/Data/workspace/angular/cses-client/tests/e2e/wdio/harness/run-context.ts` 实现 `G01Run`、`TestPersona`、lane/evidence-level tagged union 和合法状态转换
- [ ] T009 [P] 在 `/System/Volumes/Data/workspace/angular/cses-client/tests/e2e/wdio/harness/__tests__/artifact-writer.test.ts` 增加 run/persona 路径隔离、path traversal、hash/size 与首次失败保留 RED tests
- [ ] T010 在 `/System/Volumes/Data/workspace/angular/cses-client/tests/e2e/wdio/harness/artifact-writer.ts` 实现只写当前 run root 的原子 artifact/manifest writer
- [ ] T011 [P] 在 `/System/Volumes/Data/workspace/angular/cses-client/tests/e2e/wdio/harness/__tests__/correlation-index.test.ts` 增加 reqId/trackId/postId/eventSeq/transactionId 增量索引、旧 run 拒绝和 timeout RED tests
- [ ] T012 在 `/System/Volumes/Data/workspace/angular/cses-client/tests/e2e/wdio/harness/correlation-index.ts` 实现 event latch 与平均 `O(1)` 单键查询，禁止等待时全量重扫日志
- [ ] T013 [P] 在 `/System/Volumes/Data/workspace/angular/cses-client/tests/e2e/wdio/harness/__tests__/log-redactor.test.ts` 注入 password/token/authorization/Cookie/body/raw Frame/正文并建立 fail-closed RED tests
- [ ] T014 在 `/System/Volumes/Data/workspace/angular/cses-client/tests/e2e/wdio/harness/log-capture.ts` 与 `log-redactor.ts` 实现 console、Tauri/Helix、Go/WS bounded-window allowlist 采集
- [ ] T015 [P] 在 `/System/Volumes/Data/workspace/angular/cses-client/tests/e2e/wdio/harness/timing.ts` 实现 build/startup/setup/action/ack/frame/dom/evidence/screenshot/report/total 阶段计时和 warm P95 聚合
- [ ] T016 [P] 在 `/System/Volumes/Data/workspace/angular/cses-client/tests/e2e/wdio/reporters/__tests__/g01-report-v1.test.ts` 增加 Spec/JUnit/JSON/Markdown/manifest 一致性、claim boundary、exit code 与 flaky RED tests
- [ ] T017 在 `/System/Volumes/Data/workspace/angular/cses-client/tests/e2e/wdio/reporters/g01-report-v1.ts` 和 `reporters/g01-summary-reporter.ts` 实现 Report V1 聚合与附件索引
- [ ] T018 [P] 在 `/System/Volumes/Data/workspace/angular/cses-client/tests/e2e/wdio/harness/__tests__/topology-adapter.test.ts` 为 single/dual 按需实例、persona 独立恢复、普通 case 零重启增加 RED tests
- [ ] T019 在 `/System/Volumes/Data/workspace/angular/cses-client/tests/e2e/wdio/harness/topology-adapter.ts` 适配现有 `tests/e2e/desktop/harness/native-topology.ts`，不重写登录、进程或 profile 管理
- [ ] T020 [P] 在 `/System/Volumes/Data/workspace/angular/cses-client/tests/e2e/wdio/contracts/__tests__/production-boundary.test.ts` 建立 default/release 构建不得包含 WDIO plugin、capability、Mock bridge 或调试命令的 RED test

## Phase 3: User Story 1 - Test-only Safe Entry (Priority: P1)

**Goal**: 测试 build 能创建真实 WKWebView session，production build 无相同入口。

**Independent Test**: 执行 preflight/session smoke 和 production-boundary negative scan，无需 G01 真实发送。

- [ ] T021 [P] [US1] 在 `/System/Volumes/Data/workspace/angular/cses-client/src-tauri/tests/wdio_plugin_boundary.rs` 增加 test profile 注册、无 feature panic、default build 无插件的 RED tests
- [ ] T022 [US1] 在 `/System/Volumes/Data/workspace/angular/cses-client/src-tauri/src/framework/plugins.rs` 按 `wdio-e2e` feature + explicit E2E profile 双条件注册新测试插件并保持 single-instance 生产行为
- [ ] T023 [US1] 在 `/System/Volumes/Data/workspace/angular/cses-client/tests/e2e/wdio/harness/preflight.ts` 实现 macOS x86_64、Node/Rust、端口、Screen Recording、artifact、credential source、SQLite/PG readonly 和 source pin fail-fast 检查
- [ ] T024 [US1] 在 `/System/Volumes/Data/workspace/angular/cses-client/scripts/message-wdio/build-candidate.mjs` 实现一次 test candidate build 与 default/release production-boundary scan；遵守 Cargo 中途禁跑、模块完成前一次 check、阶段结束一次 build/test
- [ ] T025 [US1] 在 `/System/Volumes/Data/workspace/angular/cses-client/tests/e2e/wdio/specs/g01-safe-entry.e2e.ts` 验证 session status、当前窗口、测试能力 allowlist、未授权命令/路径拒绝和 artifact 脱敏

## Phase 4: User Story 2 - Single-instance UI, Mock And Real (Priority: P1)

**Goal**: UI-only、Mock、IPC reject、真实 integration 和 single-real 分层可证伪，互不冒充。

**Independent Test**: 一个 Actor 常驻 suite 依次运行 G01 UI、Helix-derived fixture Mock、Mock 清理后的真实 invoke 和单端真实链。

- [ ] T026 [P] [US2] 在 `crates/helix-im/tests/g01_wdio_fixture_contract_test.rs` 增加当前 G01 producer 导出 `sending/sent/failed` InteractionFrame V2、source pin、surface 集合、identity/delivery 和 semantic hash RED tests
- [ ] T027 [US2] 在 `crates/helix-im/tests/fixtures/g01-wdio/` 由 Helix serializer/producer 生成三份 canonical V2 fixture 与 `provenance.json`，禁止人工补 Surface/VM 字段
- [ ] T028 [US2] 在 `/System/Volumes/Data/workspace/angular/cses-client/scripts/message-wdio/sync-helix-fixtures.mjs` 实现只读复制与 commit/schema/file hash/semantic hash 校验到 `tests/e2e/wdio/fixtures/g01/`
- [ ] T029 [P] [US2] 在 `/System/Volumes/Data/workspace/angular/cses-client/tests/e2e/wdio/fixtures/g01/__tests__/fixture-parity.test.ts` 增加 schemaVersion=2、三终态、Helix pin、codec decode 和禁止 client-side 字段补写 tests
- [ ] T030 [P] [US2] 在 `/System/Volumes/Data/workspace/angular/cses-client/tests/e2e/wdio/harness/__tests__/tauri-mock-bridge.test.ts` 增加 `AcceptedCommand` shape、once/reset/restore、unknown command 和真实 `im:__bus__` 投递 RED tests
- [ ] T031 [US2] 在 `/System/Volumes/Data/workspace/angular/cses-client/tests/e2e/wdio/harness/tauri-mock-bridge.ts` 实现 receipt Mock 与 Frame V2 bus replay 的两个独立端口
- [ ] T032 [P] [US2] 在 `/System/Volumes/Data/workspace/angular/cses-client/tests/e2e/wdio/pageobjects/g01.page.ts` 实现 Surface ready、Composer type/submit、delivery、message identity、CSS/尺寸和 screenshot 的统一 page object
- [ ] T033 [P] [US2] 在 `/System/Volumes/Data/workspace/angular/cses-client/tests/e2e/wdio/specs/g01-ui.e2e.ts` 增加真实 WKWebView 的 ready/input/submit/disabled/loading/DOM 合同和失败截图
- [ ] T034 [P] [US2] 在 `/System/Volumes/Data/workspace/angular/cses-client/tests/e2e/wdio/specs/g01-mock-sent.e2e.ts` 验证 payload/call count、receipt、`sending -> sent`、temporary identity 收敛和 `mock` claim boundary
- [ ] T035 [P] [US2] 在 `/System/Volumes/Data/workspace/angular/cses-client/tests/e2e/wdio/specs/g01-mock-failed.e2e.ts` 验证 `sending -> failed` DOM 只由 matching Frame 驱动
- [ ] T036 [P] [US2] 在 `/System/Volumes/Data/workspace/angular/cses-client/tests/e2e/wdio/specs/g01-ipc-rejected.e2e.ts` 验证一次 invoke reject 分类为 `ipc-rejected` 且不产生业务 failed Frame
- [ ] T037 [US2] 在 `/System/Volumes/Data/workspace/angular/cses-client/tests/e2e/wdio/specs/g01-integration.e2e.ts` 清空 Mock 后执行真实 health/query/send，并关联 console、Tauri/Helix、matching Frame 与 Actor DOM
- [ ] T038 [US2] 在 `/System/Volumes/Data/workspace/angular/cses-client/tests/e2e/wdio/specs/g01-single-real.e2e.ts` 增加 Actor UI、Go HTTP、SQLite、PG、Frame、FrameApplied、Actor DOM 的单端 real-chain 断言
- [ ] T039 [US2] 在 `/System/Volumes/Data/workspace/angular/cses-client/tests/e2e/wdio/harness/failure-escalation.ts` 将单实例首次失败、DOM 摘要、console、Tauri/Helix、real lane 数据库/Go 证据和截图接入 Report V1

## Phase 5: User Story 3 - G01 Driver Migration And Efficiency (Priority: P1)

**Goal**: G01 不再直接调用 raw W3C client，且保持字段/断言等价与 warm 性能预算。

**Independent Test**: 同 marker 对比旧代表链与新 WDIO 链；等价后只让 G01 raw 调用为 0，其他 Gate 不受影响。

- [ ] T040 [P] [US3] 在 `/System/Volumes/Data/workspace/angular/cses-client/tests/e2e/wdio/contracts/__tests__/g01-raw-equivalence.test.ts` 固定旧链 selector/action/DOM/correlation/artifact 字段与新链等价 RED contract
- [ ] T041 [P] [US3] 在 `/System/Volumes/Data/workspace/angular/cses-client/tests/e2e/wdio/harness/__tests__/driver-port.test.ts` 为 selector/click/type/text/attribute/execute/screenshot/session/wait 统一 driver port 增加 tests
- [ ] T042 [US3] 在 `/System/Volumes/Data/workspace/angular/cses-client/tests/e2e/wdio/harness/wdio-driver-port.ts` 实现 driver port 并禁止暴露 raw W3C session request
- [ ] T043 [US3] 在 `/System/Volumes/Data/workspace/angular/cses-client/tests/e2e/desktop/harness/message-interaction-operations.ts` 将 G01 操作入口改为调用 WDIO adapter，保留其他 Gate 的现有 driver 路径
- [ ] T044 [US3] 在 `/System/Volumes/Data/workspace/angular/cses-client/tests/e2e/scenarios/g-01-send-text.scenario.ts` 切断 G01 对 `cses-client-desktop-driver.ts` 的直接调用并通过等价合同
- [ ] T045 [P] [US3] 在 `/System/Volumes/Data/workspace/angular/cses-client/tests/e2e/wdio/contracts/__tests__/g01-no-raw-calls.test.ts` 增加 G01 source scan=0、共享旧 driver 仍被非 G01 使用时不得删除的护栏
- [ ] T046 [US3] 在 `/System/Volumes/Data/workspace/angular/cses-client/tests/e2e/wdio/harness/event-latches.ts` 对接 WDIO、console、Tauri/Helix 与现有 evidence collector 的增量事件，不使用固定 sleep
- [ ] T047 [US3] 在 `/System/Volumes/Data/workspace/angular/cses-client/tests/e2e/wdio/harness/evidence-adapter.ts` 实现成功紧凑采集与失败全量升级，并保留 first failure/retry history
- [ ] T048 [US3] 在 `/System/Volumes/Data/workspace/angular/cses-client/tests/e2e/wdio/specs/g01-performance.e2e.ts` 验证单实例 suite 普通 case profile restart=0、Mock/UI P95≤10s、single-real P95≤15s及阶段等待原因

## Phase 6: User Story 4 - Dual-account Real G01 (Priority: P1)

**Goal**: 444 Actor 真实发送、678 Observer 真实接收，同一 run 九面证据与双端截图闭合。

**Independent Test**: dual lane 单独启动两个隔离实例；Observer 不接收测试 Frame。

- [ ] T049 [P] [US4] 在 `/System/Volumes/Data/workspace/angular/cses-client/tests/e2e/wdio/harness/__tests__/dual-topology.test.ts` 增加 profile/auth/device/port/log/artifact 隔离、单 persona 恢复和跨 run 污染 RED tests
- [ ] T050 [US4] 在 `/System/Volumes/Data/workspace/angular/cses-client/tests/e2e/wdio/config/dual-real.ts` 配置 444 Actor/678 Observer 的 multiremote/session capability 与受控 credential source
- [ ] T051 [US4] 在 `/System/Volumes/Data/workspace/angular/cses-client/tests/e2e/wdio/harness/dual-topology.ts` 编排按需双实例、独立 readiness、单 persona restart 和健康 persona 现场保留
- [ ] T052 [US4] 在 `/System/Volumes/Data/workspace/angular/cses-client/tests/e2e/wdio/specs/g01-dual-real.e2e.ts` 实现 Actor 真实 UI send 与 Observer 真实 WS/Helix/DOM receive，禁止 Observer fixture injection
- [ ] T053 [US4] 在 `/System/Volumes/Data/workspace/angular/cses-client/tests/e2e/wdio/harness/g01-correlation.ts` 闭合 reqId/trackId/channel/temporaryIdHash/postId/eventSeq/transactionId/FrameApplied/双 DOM
- [ ] T054 [P] [US4] 在 `/System/Volumes/Data/workspace/angular/cses-client/tests/e2e/wdio/harness/storage-evidence.ts` 适配本次 profile SQLite 与远端 PG 的有界只读 before/after 查询，禁止写库和 dump
- [ ] T055 [P] [US4] 在 `/System/Volumes/Data/workspace/angular/cses-client/tests/e2e/wdio/harness/dual-screenshot.ts` 生成同一 fresh run 的 Actor/Observer 终态截图、尺寸/遮挡断言和 hash 索引
- [ ] T056 [US4] 在 `/System/Volumes/Data/workspace/angular/cses-client/tests/e2e/wdio/harness/dual-evidence-reducer.ts` 强制 Go/WS/SQLite/PG/Frame/FrameApplied/Actor DOM/Observer DOM/双截图缺一非 Green
- [ ] T057 [US4] 在 `/System/Volumes/Data/workspace/angular/cses-client/tests/e2e/wdio/specs/g01-dual-recovery.e2e.ts` 验证 Observer 超时/断 session 时只恢复 Observer，Actor 现场与首次失败附件保持不变

## Phase 7: User Story 5 - Canonical Coexistence And Report V1 (Priority: P2)

**Goal**: G01 与现有 stop/restart/profile Harness 共存，并产出第一版人机可读报告。

**Independent Test**: 注入一个 pass、一个 fail、一个 retry-pass；报告、附件、脱敏、flaky 和退出码一致。

- [ ] T058 [P] [US5] 在 `/System/Volumes/Data/workspace/angular/cses-client/tests/e2e/wdio/specs/g01-report-contract.e2e.ts` 注入 pass/fail/retry-pass 并建立退出码、first failure、flaky、附件一致性 RED tests
- [ ] T059 [P] [US5] 在 `/System/Volumes/Data/workspace/angular/cses-client/tests/e2e/wdio/reporters/markdown-summary.ts` 生成按 lane/evidence level/claim boundary/失败 stage 的紧凑 Markdown
- [ ] T060 [US5] 在 `/System/Volumes/Data/workspace/angular/cses-client/tests/e2e/wdio/reporters/run-manifest.ts` 汇总 source pins、process windows、case timing、retry、artifact hash 和 redaction verdict
- [ ] T061 [US5] 在 `/System/Volumes/Data/workspace/angular/cses-client/tests/e2e/wdio/specs/g01-harness-coexistence.e2e.ts` 证明现有 stop/restart/profile 控制可与统一 driver 共存但不宣称 GS01-GS04 Green
- [ ] T062 [US5] 在 `/System/Volumes/Data/workspace/angular/cses-client/wdio.conf.ts` 接通 Spec、JUnit、G01 summary reporters、失败 hooks、suite exit code 和当前 run artifact root
- [ ] T063 [US5] 在 `/System/Volumes/Data/workspace/angular/cses-client/scripts/message-wdio/reduce-performance.mjs` 按 lane 计算 warm P95、零普通 restart、阶段耗时和超时原因
- [ ] T064 [US5] 在 `/System/Volumes/Data/workspace/angular/cses-client/package.json` 完成 `preflight/build/g01:ui/mock/integration/single-real/dual-real/report-contract/all` 稳定入口并串联五阶段 fail-fast 执行

## Phase 8: Polish And Closure

- [ ] T065 [P] 运行 Helix G01 fixture focused contract、client schema/harness/reporter tests、Tauri test-build 与 production-boundary checks，将命令、起止时间和结果写入 `specs/019-g01-wdio-five-phases/verification.md`
- [ ] T066 运行 G01 UI/Mock/integration/single-real/dual-real/Report V1 全套 lane，记录 source/candidate/run ids、P95、retries 和 artifact hashes 到 `specs/019-g01-wdio-five-phases/verification.md`
- [ ] T067 [P] 对 `artifacts/message-wdio/<runId>` 执行 forbidden key/value、绝对路径、跨 run、raw payload、历史 screenshot 和 schema validation scan，并登记 redaction verdict
- [ ] T068 [P] 更新 `/System/Volumes/Data/workspace/angular/cses-client/docs/原生桌面测试体系/06-实施路线与验收标准.md`，仅登记已验证的 G01 命令、证据等级、报告路径和未覆盖 GS/Gate
- [ ] T069 复核 G01 raw call=0 且非 G01 raw 调用未被破坏；若全仓仍有其他调用则保留共享 driver，并在 `specs/019-g01-wdio-five-phases/verification.md` 记录删除前置未满足
- [ ] T070 运行 V-001 至 V-008、`git diff --check` 与 mandatory after_implement loop；只有 R4 PASS 且 tasks/evidence fingerprint 当前才关闭 Feature

## Dependencies And Execution Order

```text
T001-T006
  -> T007-T020
  -> T021-T025
  -> T026-T039
  -> T040-T048
  -> T049-T057
  -> T058-T064
  -> T065-T070
```

- T007-T020 是所有 story 的共享阻塞基座。
- T026-T029 的 Helix fixture 真源先于任何 Mock spec。
- T040-T045 只有在新链等价测试通过后才切断 G01 raw 调用；不允许提前删除共享 driver。
- T049-T057 只在 dual lane 启动 Observer；普通 suite 不承担双实例成本。
- T065/T066 分开执行 focused/tooling 与真实 lane；build/install/首次授权不混入 warm P95。

## Parallel Opportunities

- T002-T005、T007/T009/T011/T013/T015/T016/T018/T020 可在互不写同文件时并行。
- US2 的 UI、Mock sent、Mock failed、IPC reject RED specs 可并行；真实 integration/single-real 串行复用 Actor topology。
- US4 的 storage evidence 与 screenshot helper 可并行；最终 reducer 依赖两者。
- 共享 `wdio.conf.ts`、`package.json`、`plugins.rs`、topology/evidence adapter 必须串行合并。

## Implementation Strategy

1. MVP：T001-T025，先建立不进入生产的真实 WKWebView WDIO session。
2. Single layered：T026-T039，证明 Helix-derived Mock 与真实 Tauri/Helix 分层。
3. Efficiency：T040-T048，只迁移 G01 并量化反馈时间。
4. Canonical：T049-T057，完成 444/678 双端真实链。
5. Closure：T058-T070，Report V1、脱敏、性能与 R4。

## Loop Task Contracts

| Task | Requirement | Plan Item | Change Scope | Preconditions | Validator | Done Signal |
|---|---|---|---|---|---|---|
| T001 | FR-015,FR-016 | I-001..I-003 | `verification.md` | R1/R2 PASS | source/hash scan | source lock recorded |
| T002 | FR-001 | A-001 | client package/lock | T001 | install + lock diff | WDIO family pinned |
| T003 | FR-002 | A-001 | client Cargo feature | T001 | feature metadata check | default unchanged |
| T004 | FR-003 | A-001 | Tauri capability/config | T001 | config lint | minimal allowlist |
| T005 | FR-017,FR-021 | A-003 | WDIO lane config | T002 | config unit test | six lanes defined |
| T006 | FR-022 | A-001 | WDIO test structure | T002-T005 | TypeScript include check | structure compiles |
| T007 | FR-009,FR-029 | V-002 | schema tests | T006 | focused RED | invalid contracts rejected |
| T008 | FR-017,FR-030 | S-001..S-005 | run model | T006 | model tests | illegal state impossible |
| T009 | FR-025,FR-032 | A-006 | artifact tests | T006 | focused RED | unsafe paths fail |
| T010 | FR-025,FR-032 | A-006 | artifact writer | T009 | artifact tests | atomic indexed writes |
| T011 | FR-019,FR-027 | A-003 | correlation tests | T006 | focused RED | stale/timeout fail |
| T012 | FR-019 | A-003 | correlation index | T011 | index tests | O(1) lookup |
| T013 | FR-005,FR-033 | A-006 | redaction tests | T006 | injected secrets RED | forbidden data rejected |
| T014 | FR-027,FR-033 | A-006 | log capture/redactor | T013 | log tests | bounded allowlist logs |
| T015 | FR-021 | V-008 | timing model | T006 | timing tests | stages/P95 deterministic |
| T016 | FR-029..FR-034 | A-006 | reporter tests | T007-T010 | focused RED | report mismatch detected |
| T017 | FR-029..FR-034 | A-006 | Report V1 | T016 | reporter tests | four outputs agree |
| T018 | FR-017,FR-018,FR-026 | A-003,A-005 | topology tests | T006 | focused RED | isolation failures caught |
| T019 | FR-017,FR-018,FR-026 | A-003,A-005 | topology adapter | T018 | topology tests | on-demand personas |
| T020 | FR-002 | V-003 | production guard test | T003-T006 | default build RED | test entry detected |
| T021 | FR-002 | A-001 | Rust boundary tests | T003-T004 | focused Rust RED | invalid registration caught |
| T022 | FR-001..FR-003 | A-001 | plugin registration | T021 | V-003 | test-only plugin PASS |
| T023 | FR-004..FR-006 | A-001 | preflight | T008,T014,T019 | preflight tests | exact blockers reported |
| T024 | FR-002 | A-001 | candidate builder | T022-T023 | V-003 | test/default boundary PASS |
| T025 | FR-001..FR-005 | S-001 | safe-entry spec | T024 | WDIO smoke | SafeHarnessReady |
| T026 | FR-008,FR-009 | A-002 | Helix fixture test | T001 | V-001 RED | producer gaps visible |
| T027 | FR-008,FR-009 | A-002 | Helix fixtures | T026 | V-001 | three V2 fixtures PASS |
| T028 | FR-009 | A-002 | fixture sync | T027 | hash parity | mirror exact |
| T029 | FR-008,FR-009 | A-002 | client fixture tests | T028 | codec/parity tests | pin/schema/hash PASS |
| T030 | FR-010..FR-013 | A-002 | bridge tests | T025,T029 | focused RED | receipt/frame split enforced |
| T031 | FR-008..FR-013 | A-002 | mock bridge | T030 | bridge tests | bus replay PASS |
| T032 | FR-007,FR-014 | A-003 | G01 page object | T025 | page contract tests | stable UI operations |
| T033 | FR-007,FR-014 | A-003 | UI spec | T032 | V-004 | ui-only PASS |
| T034 | FR-008,FR-010,FR-013 | A-002 | mock sent spec | T031-T032 | V-004 | mock sent PASS |
| T035 | FR-010,FR-013 | A-002 | mock failed spec | T031-T032 | V-004 | business failed PASS |
| T036 | FR-011 | A-002 | IPC reject spec | T031-T032 | V-004 | ipc-rejected PASS |
| T037 | FR-012,FR-013 | A-003 | integration spec | T033-T036 | V-004 | real invoke PASS |
| T038 | FR-012..FR-014 | A-003 | single-real spec | T037 | V-004 | single-real-chain PASS |
| T039 | FR-014,FR-020 | A-007 | failure escalation | T014,T017,T038 | injected failure | first failure retained |
| T040 | FR-015,FR-016 | A-004 | equivalence test | T038 | contract RED | missing field detected |
| T041 | FR-015 | A-004 | driver port tests | T032 | focused RED | raw access rejected |
| T042 | FR-015 | A-004 | WDIO driver port | T041 | port tests | unified API PASS |
| T043 | FR-015 | A-004 | G01 operations adapter | T040,T042 | equivalence test | G01 uses adapter |
| T044 | FR-015,FR-016 | A-004 | G01 scenario | T043 | V-005 | representative chain PASS |
| T045 | FR-016 | A-004 | no-raw guard | T044 | source scan | G01 raw=0 |
| T046 | FR-019 | A-003 | event latches | T012,T042 | latch tests | fixed sleep=0 |
| T047 | FR-020,FR-032 | A-006,A-007 | evidence adapter | T039,T046 | evidence tests | compact/escalated split |
| T048 | FR-017..FR-021 | V-008 | performance spec | T044-T047 | timing reducer | P95/restart budgets PASS |
| T049 | FR-022,FR-026,FR-027 | A-005 | dual topology tests | T019 | focused RED | collisions caught |
| T050 | FR-022 | A-005 | dual config | T049 | config lint | personas isolated |
| T051 | FR-022,FR-026 | A-005 | dual topology | T050 | topology tests | per-persona recovery |
| T052 | FR-023 | A-005 | dual G01 spec | T038,T051 | V-006 | real send/receive |
| T053 | FR-024,FR-027 | A-005 | G01 correlation | T012,T052 | correlation contract | identity chain closed |
| T054 | FR-006,FR-024 | A-005 | storage evidence | T052 | readonly query tests | bounded DB deltas |
| T055 | FR-025 | A-005 | dual screenshots | T052 | freshness/layout test | fresh images indexed |
| T056 | FR-024,FR-025 | A-007 | dual reducer | T053-T055 | V-006 | nine planes required |
| T057 | FR-026 | A-007 | dual recovery spec | T051,T056 | failure injection | only damaged persona restarts |
| T058 | FR-029..FR-034 | A-006 | report E2E contract | T017,T039 | focused RED | inconsistency detected |
| T059 | FR-029..FR-031 | A-006 | Markdown summary | T058 | snapshot/schema test | claim boundary visible |
| T060 | FR-032..FR-034 | A-006 | run manifest | T058 | schema/redaction test | artifact index complete |
| T061 | FR-028 | A-003,A-005 | coexistence spec | T057 | WDIO scenario | G01 only claim |
| T062 | FR-029..FR-032 | A-006 | WDIO reporters | T059-T060 | V-007 | outputs/exit aligned |
| T063 | FR-021 | V-008 | performance reducer | T048,T057 | P95 contract | lane budgets reported |
| T064 | FR-029 | A-006 | package commands | T061-T063 | command smoke | stable five-phase entry |
| T065 | SC-001,SC-007,SC-010 | V-001..V-003 | focused verification | T001-T064 | listed commands | contracts/tooling PASS |
| T066 | SC-001..SC-012 | V-004..V-008 | real-lane verification | T065 | G01 all command | five phases PASS |
| T067 | SC-008 | V-007 | artifact security scan | T066 | secret/schema scanner | redaction PASS |
| T068 | FR-028,FR-029 | A-006 | testing docs | T066 | doc/source link check | commands documented |
| T069 | FR-016 | A-004 | raw migration audit | T066 | source/reference scan | G01=0, others preserved |
| T070 | SC-001..SC-012 | V-001..V-008 | feature closure | T065-T069 | loop R4 | Feature Closed |
