# Tasks: UC04-16 Native E2E 收敛修复

**前置**: 004-016 focused tasks 已完成或缺项已登记。017 必须修复并重验，不得在产生 Red/Partial 报告后停止。

## Phase 1: Freeze and Preflight

- [X] T001 汇总 004-016 readiness 与 UC04/G03a 回归，逐 Gate 检查 ActionBinding、Tick、Persist、Surface/VM、Frame、Angular/UI。
- [X] T002 记录 Helix、Go server、cses-client revision、dirty worktree、consumer pin、fixture manifest 和分支身份；为需要修改的 owner 分别建立独立 worktree，不得覆盖既有修改。
- [X] T003 校验 444/678、company、Go/PG、两 profile SQLite、日志目录、设计 URL 与内部浏览器能力；密码只从安全环境注入。
- [X] T004 创建 runId、证据目录和 Gate result schema，使 HTTP/WS/DB/Persist/Frame/DOM/screenshot 可按 causation 关联。

## Phase 2: Contract and Candidate Build

- [X] T005 运行 doctor、source-pin、strict-contract、Helix focused contract/replay；批量修复静态缺口后再进入 build。
- [X] T006 一次运行 Helix 五 crate check、core/im tests、Web final build，记录 start/end/wall-clock 和 cache。
- [X] T007 re-pin cses-client 后构建首个 Native candidate；不为单个 UC 单独 build。

## Phase 3: Continuous UC04-16 Run

- [X] T008 启动 444/678 双 profile、真实 Go 链和结构化日志采集，保存 connectionId/trackId/corr_key/causationId。
- [X] T009 连续执行 UC04-UC09：重发、撤回、转发、reaction、加急、定时创建/取消。
- [X] T010 连续执行 UC10-UC14：窗口、定位、回复树、频道/消息已读、回执、模板 received。
- [X] T011 连续执行 UC15-UC16：建群、话题、关闭、自退群、成员与设置治理。
- [X] T012 为每个 Gate 生成 actor/observer 的 HTTP、WS、remote PG、local SQLite、Frame、DOM、visual 初始 verdict。

## Phase 4: Diagnose

- [X] T013 按第一处 authority divergence 分类失败：action/tick、Go HTTP/WS、normalize、Persist、Frame、apply/DOM、visual。
- [X] T014 读取当前 runId 时间窗 Go server、Helix host/native、Tauri、Angular 日志，禁止注入无关历史原始日志。
- [X] T015 使用远端 PostgreSQL 与 444/678 本地 SQLite 对账 entity、member、receipt、schedule、cursor、revision 和 viewer isolation。
- [X] T016 形成失败台账：Gate、复现步骤、layer、owner、root cause、affected Gates、修复文件、回归命令；禁止只写“E2E failed”。

## Phase 5: Red Reproduction

- [X] T024 在任何 owner 生产修复前，先为本轮 failure signature 新增并运行可证伪回归，保存失败输出；同一 task/validator/失败签名最多两次自动修复，第三次进入 NeedsHuman。

## Phase 6: Repair by Owner

- [X] T017 修复 Helix ActionBinding/Tick/capability/action ledger 缺口，并补 deterministic replay/contract test。
- [X] T018 修复 Helix normalize/PersistAtomic/cursor/revision/Surface/VM/Frame 缺口，并补 PersistOk barrier、重复/乱序和 viewer isolation 回归。
- [X] T019 修复 cses-im-server handler/application/transaction/WS audience/eventSeq 根因，并补 Go 可证伪测试。
- [X] T020 修复 cses-client Tauri Frame delivery/source pin/codec/apply 根因，保持 transport-only 与 whole-frame apply。
- [X] T021 修复 Message-v4 Angular component、DOM、ARIA、稳定 data-*、失败恢复与业务交互，禁止回退 raw event/legacy reducer。
- [X] T022 对照 Index-Dual 修复 final Native 桌面样式、overlay、菜单、抽屉、banner、交互状态；关键项不得以 waiver 代替实现。
- [X] T023 公共 contract 或 wire 改动时同步 Helix fixture/schema、Go contract、cses-client codec/source pin 和相关 Spec task 状态。

## Phase 7: Repair-Batch Verification

- [X] T025 聚合本轮修改后，每个 repair batch 最多执行一次受影响 target/Native 增量 rebuild，不按 UC 重编。
- [X] T026 重跑失败 Gate 与直接依赖 Gate；记录修复前后 causation、DB、Frame、DOM 差异。
- [X] T027 若修改 shared contract、Persist、Frame、SurfaceRuntime 或 viewer isolation，立即重跑 T008-T012 全批。
- [X] T028 若仍有 required fail/skip、证据不一致或关键 visual partial，回到 T013；不得进入 Seal。

## Phase 8: Final Candidate

- [X] T029 在最后一次代码修复后冻结 final Helix/Go/cses-client pin，执行一次 final Native rebuild。
- [X] T030 在 final candidate 上连续重跑 UC04-UC16，要求 required fail=0、required skip=0。
- [X] T031 对 final run 重新核对双账号 remote PG、local SQLite、完整 Frame、DOM/ARIA/interaction，历史 run 不得代替。
- [X] T032 用内部浏览器回放 Index-Dual，并在 actor/observer 的 1440x900 final Native 视口复验真实 Message-v4；关键项全部 aligned。

## Phase 9: Seal

- [X] T033 生成 UC04-16 Strict Matrix，逐 Gate 链接 contract、Go、PG、SQLite、Frame、DOM、visual 和 repair diff 哈希。
- [X] T034 记录总任务数、完成/剩余、全部命令 start/end/wall-clock、setup/cache/retry/timeout/skip/block。
- [X] T035 仅在 final pin 的 G04-G16 全部 Full Green 且未修失败数为 0 时写完成；否则回到 T013。

## Execution Loop

`T008-T012 -> T013-T016 -> T024(red) -> T017-T023(repair) -> T025-T028 -> T008-T012` 持续循环；同一失败签名两次自动修复仍未通过则进入 NeedsHuman。只有 T028 判定零未解决问题后才能进入 T029-T035。

## Loop Task Contracts

| Task | Requirement | Plan Item | Change Scope | Preconditions | Validator | Done Signal |
|---|---|---|---|---|---|---|
| T001 | FR-001 | R0 | 三仓 readiness 与 Gate contract | 现有 004-016 | manifest inspection | 逐 Gate 缺项明确 |
| T002 | FR-009 | R0 | 三仓 revision/worktree/pin | T001 | git/source-pin inspection | 身份冻结且不覆盖脏改动 |
| T003 | FR-009 | R0 | 账号/DB/log/browser preflight | T002 | capability preflight | 真实前置可用或明确阻塞 |
| T004 | FR-003 | R0 | run manifest/evidence schema | T003 | schema validation | 全证据可按 runId 关联 |
| T005 | FR-001 | R0 | contract/doctor/static repair | T004 | doctor/source-pin/contracts | 静态缺口清零 |
| T006 | FR-005 | R0 | Helix final batch | T005 | cargo checks/tests/build | 首批 Rust 结果有时长 |
| T007 | FR-005 | R0 | cses-client candidate build | T006 | desktop build | 未按 UC 重编 |
| T008 | FR-002 | R1 | 双 profile/Go/log processes | T007 | connection/log probes | 双账号连接可追踪 |
| T009 | FR-002 | R1 | UC04-UC09 scenarios | T008 | scenario result | 前半 Gate 有结构化 verdict |
| T010 | FR-002 | R1 | UC10-UC14 scenarios | T009 | scenario result | 中段 Gate 有结构化 verdict |
| T011 | FR-002 | R1 | UC15-UC16 scenarios | T010 | scenario result | 后段 Gate 有结构化 verdict |
| T012 | FR-007 | R1 | 六面初始证据 | T009-T011 | evidence collector | 每 Gate verdict 完整 |
| T013 | FR-003 | R2 | failure layer classification | T012 | first-divergence analysis | 失败归属唯一 owner |
| T014 | FR-003 | R2 | 当前 run 日志 | T013 | correlation search | 日志与 runId 对齐 |
| T015 | FR-003 | R2 | remote PG/local SQLite | T013 | DB assertions | authority/persist 差异定位 |
| T016 | FR-003 | R2 | failure ledger | T014-T015 | ledger validation | root cause/repair set 可执行 |
| T017 | FR-004 | R3 | Helix action/tick | T016,T024 red | new replay/contract test | action 根因修复 |
| T018 | FR-004 | R3 | Helix persist/frame | T016,T024 red | persist/frame regression | Frame 根因修复 |
| T019 | FR-004 | R3 | Go server authority | T016,T024 red | Go test | Go 根因修复 |
| T020 | FR-004 | R3 | Tauri transport/codec/apply | T016,T024 red | Rust consumer test | PC 接缝根因修复 |
| T021 | FR-004 | R3 | Angular behavior/DOM | T016,T024 red | component/DOM test | 交互根因修复 |
| T022 | FR-008 | R3 | Index-Dual UI | T016,T024 red | dual-profile Native visual test | 关键视觉交互修复 |
| T023 | FR-006 | R3 | cross-repo contract/pin | T017-T022 | fixture/schema/source-pin gates | 生产者消费者同步 |
| T024 | FR-004 | R3 | owner-specific regression file + `evidence/red/` | T016 | failing command output | 修复前红灯可证伪；超预算 NeedsHuman |
| T025 | FR-005 | R4 | repair-batch build | T024 | affected build | 每批最多一次 rebuild |
| T026 | FR-006 | R4 | failed/dependent Gates | T025 | targeted rerun | 局部回归全绿 |
| T027 | FR-006 | R4 | shared-impact full batch | T026 | UC04-16 rerun | 共享改动无回归 |
| T028 | FR-010 | R4 | loop decision | T026-T027 | unresolved count | 零问题或回到 T013 |
| T029 | FR-005 | Final | final pins/build | T028=zero | final build | 最终 candidate 冻结 |
| T030 | FR-006 | Final | final UC04-16 batch | T029 | full Native E2E | required fail/skip=0 |
| T031 | FR-007 | Final | final six-face evidence | T030 | PG/SQLite/Frame/DOM checks | 最终证据同窗一致 |
| T032 | FR-008 | Final | final Index-Dual parity | T030 | actor/observer 1440x900 | 关键项 aligned |
| T033 | FR-007 | Seal | Strict Matrix | T031-T032 | matrix validation | 逐 Gate 哈希可追溯 |
| T034 | FR-007 | Seal | duration/evidence ledger | T033 | ledger validation | 所有耗时与重试完整 |
| T035 | FR-010 | Seal | completion decision | T033-T034 | zero unresolved gate | Full Green 或回到 T013 |
