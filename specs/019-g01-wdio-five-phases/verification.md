# Verification Procedure

## Implementation Source Lock

- Captured: `2026-07-24T12:05:48-0700`
- Helix: branch `main`, HEAD `9e595ef704624e56c805342fbbf324c339ed135e`
  - pre-implementation dirty paths: `SESSION.md`、`review-ledger.json`（本次 implement 入场写入）
  - `Cargo.lock` SHA-256: `5a110843a7dc961126a3826e2f0666f48dd74a82b1a1a1241a9997528f3d586f`
  - existing V2 manifest SHA-256: `30732bb642000e37b97ec24b07ac3334a8e9da358b83e10fc837133e99740bdd`
- cses-client: branch `feat/helix-reactor-im`, HEAD `2ba5a8de864836a1436568184d15809df72bf570`
  - pre-implementation worktree: clean
  - Helix four-package pin: `32e3a0659e4f29add1284be2ba1972eb28f0f938`
  - `yarn.lock` SHA-256: `aeab665883bf7af226212773defe73c2eb77ed00510b0175c3939f00104972ad`
  - `src-tauri/Cargo.lock` SHA-256: `9da0f79cd4d762f4345e593e25aec065c13656a7726977d369e0104928cc3a24`
  - initial G01/raw-driver candidate references: `53`（G01 scenario + shared native actions inventory；T040-T045 后重新分类，不能按名称直接删除共享 driver）
- Official package discovery: `@wdio/tauri-service`、`@wdio/tauri-plugin`、`tauri-plugin-wdio`、`tauri-plugin-wdio-webdriver` current compatible family `1.2.0`.

## Required Runs

- **V-001 Production Boundary**: `yarn message:wdio:production-boundary`
  - Expected: production artifact contains zero WDIO plugin, capability, Mock bridge or test log ingress.
- **V-002 Mock/UI Lane**: `yarn message:wdio:g01:mock`
  - Expected: UI-only、`sending -> sent`、`sending -> failed`、IPC reject and reset/restore cases all pass within the declared budget; result level never exceeds `mock`.
- **V-003 Real Integration Lane**: `yarn message:wdio:g01:integration`
  - Expected: no Mock remains; health、read-only query and send command execute through real Tauri/Helix; matching Frame and DOM are present with correlated console and Tauri logs.
- **V-004 Single Real-Chain Lane**: `yarn message:wdio:g01:single-real`
  - Expected: Actor UI、Go HTTP、SQLite、PG、Frame、FrameApplied and Actor DOM correlate; P95 scenario time is at most 15 seconds.
- **V-005 Dual Real-Chain Lane**: `yarn message:wdio:g01:dual-real`
  - Expected: isolated 444/678 instances correlate Actor send and Observer receive across all required evidence planes; P95 scenario time is at most 30 seconds.
- **V-006 Report V1 Consistency**: `yarn message:wdio:report-contract`
  - Expected: Spec/JUnit/summary/manifest counts and process exit code agree; one injected failure retains screenshot and redacted dual-side logs.
- **V-007 Performance Contract**: `yarn message:wdio:g01:performance`
  - Expected: Mock P95 <= 10 seconds, single-real P95 <= 15 seconds, dual-real P95 <= 30 seconds, one topology launch, zero ordinary profile restarts, and every slow case has phase attribution.
- **V-008 Sensitive Data Scan**: `yarn message:wdio:artifact-redaction`
  - Expected: zero password、token、authorization、Cookie、raw business payload or unapproved message content occurrences.

## Evidence Rules

- Store raw command output beneath `evidence/` and register each run with
  `loop_engine.py record-evidence`; do not replace a failed output with prose.
- Each file records command、candidate/pin、run id、start/end、wall time、result、
  retry、skip and blocker.
- Mock、integration、single-real and dual-real evidence IDs are independent and
  cannot satisfy one another.
- A failed first attempt remains immutable even if a later retry passes.
- SQLite and PG evidence is read-only and belongs to the same fresh run correlation.
- Reporter attachments contain only allowlisted, redacted summaries; raw sensitive
  logs are not copied into the report tree.

## Implementation Progress - 2026-07-24

### Focused Contracts And Builds

- `cargo test -p helix-im --test g01_wdio_fixture_contract_test`: PASS，1/1；三份
  fixture 均由当前 `ImModule::handle` producer 输出，client mirror hash 校验 PASS。
- `yarn tsc -p tests/e2e/wdio/tsconfig.json --noEmit`: PASS。
- `yarn message:wdio:contracts`: PASS，18 suites /
  58 tests；包含 schema、fixture、Mock bridge、artifact、correlation、topology、
  driver port、storage、report 和 production boundary。
- `cargo test --manifest-path src-tauri/Cargo.toml --test wdio_plugin_boundary
  --features wdio-e2e`: PASS，4/4。
- `node scripts/message-wdio/build-candidate.mjs --check`: PASS；默认 Cargo feature、
  默认 Tauri capability/config 和默认 Angular invoker 均不包含 WDIO/Mock 入口。
- `yarn ng build -c wdio-e2e --base-href /`: PASS；Mock-aware command invoker 仅由
  `wdio-e2e` file replacement 进入测试 bundle。
- `git diff --check`: PASS（本节登记前执行，收尾再次执行）。

### Passing Lanes

- `yarn message:wdio:g01:mock`: PASS，3/3；`sending -> sent`、
  `sending -> failed` 与 `ipc-rejected` 分层成立。
- `yarn message:wdio:g01:integration`: PASS；真实 Tauri invoke、真实 Helix Frame
  与 Actor DOM 闭合；coexistence PASS。性能 case 因尚无完整 real-chain 样本保持
  skipped，不冒充性能 Green。
- `yarn message:wdio:g01:report-contract`: PASS，1/1；Spec/JUnit/JSON/Markdown
  基础输出一致。
- `node scripts/message-wdio/scan-artifacts.mjs
  dual-real-index-20260724211947`: PASS，10 files，`redactionVerdict=pass`。

### Local Go Real-chain - Network Restored

- 服务端真源是本地进程
  `/System/Volumes/Data/workspace/golang/cses-im-server`，PID `3504`，
  监听 `8066`；Telepresence 仅提供 PG/Redis/Pulsar 依赖网络，
  不是被测 Go Server。
- `kubectl get pods -n cses`、Telepresence status、集群 PG `5432`、Redis
  `6379`、Pulsar `6650` 前置均 PASS。
- single-real canonical run：
  `g01-local-go-single-probe-r5-20260724154912`，1/1 PASS，Report exit `0`；
  Report V1 已索引 Go HTTP、SQLite/PG、Frame/FrameApplied、Actor DOM 三份
  脱敏证据。
- dual-real canonical run：
  `g01-local-go-dual-probe-20260724155033`，2/2 PASS；444 Actor 真实 UI
  send，678 Observer 经真实 WS/Helix/DOM 收到同一 canonical message，九面
  reducer与全字段 correlation PASS。
- dual Report V1 已索引 6 个附件：Actor/Observer 截图、截图 hash index、
  Go HTTP/WS evidence、real-chain 九面证据、SQLite/PG identity；Report exit `0`。
- redaction/schema scan：single `7 files` PASS；dual `13 files` PASS。
- Go probe 直接证明 `/api/cses/posts/create` status `200`，且双端 run 中
  678 的 `post` WS delivery 与 HTTP 使用同一 `Cses-Track-Id`。
- `@wdio/tauri-service` 在 embedded provider 下仍打印 standalone
  `tauri-driver not found` 诊断，但 embedded session 实际 PASS；不安装会改变
  provider 行为的 standalone driver。

### Remaining Stop Conditions

- T001-T070 均已执行；V-001 至 V-008 与双仓 `git diff --check` 最终通过。
- Mandatory `after_implement` 已返回 R4 `PASS`；convergence 无新增任务，
  mandatory `after_converge` 于 `event_seq=31` 返回 `CLOSED`。
- 其他 Gate 仍有 15 个 `sendTextThroughUi` 调用，因此共享 raw driver 继续保留；
  G01 scenario 与 WDIO files 的 direct raw reference scan 为 `0`。

## Final G01 Five-phase Runs - 2026-07-24

- `yarn message:wdio:all`: PASS；contracts 20 suites/60 tests、preflight、
  production boundary、candidate build、safe-entry、UI、Mock 3/3、
  integration 2/2、single-real、dual-real 2/2、Report V1 和最终 redaction
  scan 全部串行通过。performance-only contract 在未注入样本时按合同 skip，
  随后使用同一 candidate 的 fresh report 输入 reducer 验证。
- 全套关键 run：
  - UI `ui-20260724233734050-38987`
  - Mock `mock-20260724233845046-39736`
  - integration `integration-20260724233959863-40495`
  - single-real `single-real-20260724234224668-42009`
  - dual-real `dual-real-20260724234335401-42588`
  - report `report-20260724234510802-43834`
- 将 startup、初始 projection sync 和 process metadata 移出 warm case 后，
  fresh performance runs 全绿：
  - UI `g01-ui-perf-final-20260724164604`: P95 `1194ms`，预算 `10000ms`
  - Mock `g01-mock-perf-final-r4-20260724164707`: 3 samples，
    P95 `1667ms`，预算 `10000ms`
  - single-real `g01-single-perf-final-20260724164827`: P95 `3085ms`，
    预算 `15000ms`
  - dual-real `g01-dual-perf-final-20260724164938`: 2 samples，
    P95 `19311ms`，预算 `30000ms`
  - 四条 lane 均 `profileRestarts=0`、`ordinaryRestartFree=true`。
- fresh performance artifact scans：UI `5`、Mock `5`、single-real `8`、
  dual-real `11` files，全部 `redactionVerdict=pass`。
- single/dual fresh real runs继续证明本地 Go `127.0.0.1:8066`、Actor SQLite、
  远端 PG、Frame/FrameApplied 和 DOM；dual 额外闭合 Observer WS/SQLite/DOM
  与双方 fresh screenshots。
- G01 driver migration：canonical `G01Page` 通过 `WdioMessageUiDriver` 调用统一
  `submitG01TextThroughUi`；旧全 Gate scenario 只经兼容 adapter 调同一 operation。
  G01 direct raw references `0`，非 G01 仍有 `15` 个发送 helper 引用，故共享 driver
  未删除且未受破坏。

## Final Verification Registration - 2026-07-25

- V-001 至 V-008 的最终 PASS 输出已保存到 `evidence/` 并通过
  `loop_engine.py record-evidence` 登记，event sequence 为 11-18。
- 本次最终 real-chain run：
  - Mock `mock-20260724235459401-50059`
  - Integration `integration-20260724235617280-50859`
  - Single real `single-real-20260724235902280-52084`
  - Dual real `dual-real-20260725000022820-52638`
  - Report `report-20260725000223162-54259`
- V-004 首次因未向命令进程注入只读 PG DSN 而失败；失败日志原样保留，
  retry 1 在本地 Go `127.0.0.1:8066` 下 PASS。
- V-008 首次扫描进一步发现 WDIO 官方 JUnit 会在失败栈中写入本机绝对路径；
  新增 JUnit 收尾脱敏器及合同测试后，fresh failure proof
  `g01-redaction-failure-proof-20260725` 与全部 canonical/performance run
  均通过扫描。
- 最终 focused 验证：
  - Helix fixture contract：1/1 PASS。
  - cses-client TypeScript：PASS。
  - WDIO contracts：21 suites / 61 tests PASS。
  - production boundary：PASS。
  - 双仓 `git diff --check`：PASS。
- Loop 已重新校验变更后的 R1/R2/R3，并在任务最终指纹下再次返回 R4
  `PASS` (`event_seq=29`)；convergence 无新增任务，最终
  `event_seq=31`、phase=`closed`、verdict=`CLOSED`。
