# Implementation Plan: UC04-16 Native E2E 收敛修复

**Spec**: [spec.md](./spec.md) | **Type**: cross-repo convergence, repair and final acceptance

## Summary

017 运行一个持久的 `run -> classify -> repair -> rebuild -> verify -> rerun` 收敛循环。它复用同一组双账号和进程连续执行 G04-G16，避免每个 UC 重复构建；失败按真实 owner 修改 Helix、Go server 或 cses-client，直到最终 candidate 全批 Green。

## Constitution Check

*GATE: 017 只在 004-016 R1-R3 当前 PASS 后执行；任何 required evidence 缺失均不得 Seal。*

- **I Runtime-Agnostic Layering - PASS**: Helix domain 不依赖 DB/HTTP/UI；Go 与 PC 修复留在各自 owner。
- **II Deterministic Persistence - PASS**: 首个 authority divergence、PersistAtomic/PersistOk、cursor/revision 与 causation 全链可回放。
- **III InteractionFrame V2 - PASS**: Frame 是 PC 业务渲染唯一真源；禁止 raw event/legacy reducer。
- **IV Evidence Before Completion - PASS**: final candidate 必须同时具有 Go/PG、SQLite、Frame、DOM 与 visual 证据，单面 PASS 不晋级。
- **V Minimal Governed Repair - PASS**: 只修与失败 Gate 根因直接相关的生产路径；先红灯、后最小修复、再影响面回归。
- **VI Loop Budget - PASS**: 同一 task/validator/失败签名最多两次自动修复；第三次进入 NeedsHuman。
- **Safety - PASS**: 三仓脏改动保留，凭据脱敏，数据库验收只读，不做不可逆迁移。

## Repository Isolation

- Helix、cses-im-server、cses-client 分别记录 HEAD/branch/upstream/dirty state，并为需要修改的 owner 建独立 worktree；禁止一个 worktree 跨仓承载修复。
- 每个 repair batch 先冻结 owner/file set；公共 contract 变更按 Helix producer、Go authority、PC consumer/pin 顺序同步。
- 合并回当前工作树前先检查用户既有修改；冲突时协作适配，不 reset/checkout 覆盖。

## Loop Detail Contract

### Inputs and Context

- **I-001**: 004-016 readiness manifest、G04-G16 scenario matrix、三仓 revision/dirty state 与 current consumer pin。
- **I-002**: 444/678 两个隔离 profile、company scope、Go/PG、两份 SQLite、当前 runId 日志和 Index-Dual 基准。
- **I-003**: 每 Gate 的 action/HTTP/WS/PG/SQLite/Frame/DOM/visual 结构化 verdict；密码与 DSN 不进入 artifact。

### Action Chain

- **A-001**: preflight -> static contracts -> 一次 candidate build -> 双用户连续 UC04-16 -> 六面 verdict。
- **A-002**: 首个 authority divergence -> failure signature -> 红灯复现 -> owner 最小修复 -> focused verify -> repair-batch rebuild。
- **A-003**: 共享 contract/Persist/Frame/SurfaceRuntime 变化 -> 全批重跑；无共享影响时重跑失败 Gate 与直接依赖。
- **A-004**: 同一失败签名两次自动修复仍失败 -> NeedsHuman；不得第三次自动 repair。

### State Contract

- **S-001**: `Prepared -> CandidateBuilt -> Running -> Diagnosing -> RedTest -> Repairing -> Verifying`。
- **S-002**: 验证通过回到 `Running`；required fail/skip、证据冲突或关键 visual partial 非零不得进入 Seal。
- **S-003**: 零未决问题 -> `FinalCandidate -> FullBatchGreen -> Sealed`；超预算 -> `NeedsHuman`。

### Validators

- **V-001**: readiness/contract/source-pin/dirty-worktree preflight，缺一项即不启动 Native。
- **V-002**: 红灯测试必须在修复前失败、修复后通过；只改 expectation 不合格。
- **V-003**: final candidate UC04-16 required fail/skip=0，并有双用户 PG/SQLite/Frame/DOM 同窗证据。
- **V-004**: 1440x900 与 390x844 复验 Index-Dual 关键视觉与交互；关键 partial=0。
- **V-005**: evidence ledger 校验命令、起止时间、wall-clock、哈希、retry/timeout/NeedsHuman 记录完整。

## Domain and Ownership

| Failure layer | Primary owner | Repair boundary |
|---|---|---|
| ActionBinding、Tick、capability、action ledger | Helix `helix-im` | domain/application，不把平台逻辑带入 core |
| Go request/response、WS audience、远端 PG 事实 | cses-im-server + Helix Adapter | Go 修业务事实；Helix 只做协议归一 |
| PersistAtomic、cursor/revision、Surface impact、VM/Frame | Helix | PersistOk 前零业务 Frame |
| Tauri transport、consumer pin、Frame delivery | cses-client `src-tauri` | 只搬运 typed batch，不建第二业务 store |
| SurfaceRuntime、Angular component、DOM/ARIA | cses-client Message-v4 | 机械 apply/render，不消费 raw event |
| Index-Dual 样式与交互 | cses-client Message-v4 | 设计只裁视觉；业务状态仍来自 Frame |

依赖方向保持 `Angular/Tauri/driver -> Helix ports/domain`，Go wire 通过 Adapter 进入 Helix。修复不得让 domain 依赖 HTTP、DB、Tauri 或 Angular。

## Candidate and Build Strategy

1. 记录初始 Helix/Go/cses-client revision 和工作树，不覆盖既有修改。
2. 先跑 contract/doctor/source-pin，批量修完静态缺口。
3. 对一组相关 Gate 完成代码修改后，只运行一次受影响 crate/test batch。
4. Native E2E 开始前构建一个 candidate；同一 candidate 连续跑所有 UC。
5. E2E 发现问题后，先聚合成一个修复批次再增量 rebuild；禁止“一条 UC 一次 cargo”。
6. 最后一处代码修复后，生成 final candidate，执行一次完整 Native rebuild 和 UC04-UC16 全批。

“一次性 E2E”表示每个 candidate 一口气跑完整场景，不表示发现代码问题后禁止必要的 rebuild。

## Convergence Loop

### R0 Preflight

- 校验账号、company、Go/PG、SQLite 路径、设计页面、日志目录、source pin 和 dirty worktree。
- 为本轮生成 runId，所有请求、日志、DB snapshot、Frame 与 screenshot 都绑定该 runId/causation。

### R1 Execute

- 复用 444/678 两个持久连接与 Native profile。
- 顺序执行 UC04-09、UC10-14、UC15-16，不在 Gate 之间重启或重编。
- 每个 Gate 输出结构化 result：action、HTTP、WS/sync、remote commit、local persist、Frame、DOM、visual。

### R2 Diagnose

- 先找第一处 authority divergence，禁止从最终 DOM 反向猜全部业务层。
- 读取当前 runId 时间窗内 Go、host/native、Tauri、Angular 日志。
- 使用远端 PG 与本地 SQLite MCP/查询对账；任何凭据输出必须脱敏。
- 形成 `failure -> layer -> owner -> root cause -> affected Gates -> repair set`。

### R3 Repair

- Helix 修复必须保持 Sans-IO、确定性、PersistOk barrier、完整 Frame 和 O(1)/O(log n) entity lookup。
- Go 修复必须补 handler/application/WS/DB 可证伪测试，不能只改响应文案。
- cses-client 修复必须保留 SurfaceRuntime authority，补 component/DOM/ARIA/visual test，不能回退 legacy store。
- 触及公共合同必须同步 fixture/schema/source pin 和受影响 consumer tests。

### R4 Verify

- 先跑新增红灯测试确认可证伪，再运行受影响 focused tests。
- 增量 rebuild 后重跑失败 Gate 和依赖 Gate。
- shared contract/Persist/Frame/SurfaceRuntime 或 viewer isolation 改动一律触发 UC04-16 全批。
- 循环直至零 required failure/skip 和零未裁决 Index-Dual 关键差异。

## Commands

    cargo check -p helix-core -p helix-im -p helix-driver-native -p helix-driver-ffi -p helix-driver-web
    cargo test -p helix-core
    cargo test -p helix-im
    cargo build --target wasm32-unknown-unknown -p helix-driver-web
    HELIX_COOKIE_ID=444 RECEIVER_ID=678 HELIX_COMPANY_ID=64118eebd2b665246b7880eb bash scripts/host-cli-FULL-uc-e2e.sh
    npm run message:desktop:build
    npm run message:desktop:scenario:all

执行时以当前仓 scripts 为准。每条 build/test/E2E 记录 start、end、wall-clock、cache/setup、retry、timeout、skip 和 blocker。

## Evidence and Repair Matrix

| Evidence | Green condition | Typical repair |
|---|---|---|
| Contract | action/schema/pin parity | Helix contract + client codec |
| Go HTTP/WS | authority、audience、eventSeq 正确 | Go application/handler 或 Helix Adapter |
| Remote PG | canonical entity/member/receipt/schedule committed | Go transaction/domain |
| Local SQLite | actor-scoped entity/cursor/revision atomic | Helix Persist write-set/storage |
| Frame | full VM、causation、revision、epoch、atomic impact | Helix projector/composer |
| PC DOM | Frame-only state、ARIA/data-*、interaction | Angular SurfaceRuntime/component |
| Visual | Index-Dual 双尺寸关键项 aligned | Angular template/SCSS/overlay |

## Visual Repair

- 设计地址: `http://localhost:63347/message/pages/message/uc08-26/index-anchored.html`
- Handoff: `/System/Volumes/Data/workspace/pd/message/docs/message-v4-real-chain-ui/message-v4-index-dual-ui-completion-backlog.md`
- 每项执行“设计入口 -> 操作 -> 可见结果 -> 关闭/恢复”，再在真实 Message-v4 重放。
- `partial/missing/contract-drift` 必须回到 R2/R3 修复；仅非关键像素差异可在有理由和截图证据时 waiver。

## Complexity and Cost

- Native build 不按 UC 执行；按 repair batch 执行，最终 candidate 再全量一次。
- 两账号连接和进程跨 UC 复用；场景复杂度 O(UC + evidence queries)。
- correlation 索引查询 O(1)/O(log n)，Surface compose O(visible)，禁止全库扫描和 Angular 模板 O(n²)。
