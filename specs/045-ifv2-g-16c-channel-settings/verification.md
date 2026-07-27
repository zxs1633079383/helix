# Verification Procedure: G-16c

## Required Runs

- **V-001 Helix build**: plan 必须锁定受影响 crate 并执行对应 `cargo check`；期望退出码 0，且 helix-core 运行时依赖检查仍为空。
- **V-002 Helix scenario**: 执行本 Gate 的生产 `ImModule::handle` 回放测试；期望 prerequisites、happy、failure、race/idempotency 与 recovery 的 required case 全通过。
- **V-003 Canonical Fixture**: 普通测试只生成 actual Frame 并与 checked-in JSON 比较；期望 typed validation、字段断言、raw/semantic hash 与 surface inventory 全一致，未设置显式 update 开关时不得改写 Golden。
- **V-004 PC Mock**: 执行本 Gate 的 Fixture parity、Tauri Mock replay、SurfaceRuntime、DOM 断言与 reporter；期望 required fail=0/skip=0，且报告明确标记为 Mock。
- **V-005 PC Real**: 经真实 MessageV4 UI、Tauri invoke、Helix/SQLite/HTTP/WS/bus ingress 执行 single-real；涉及 observer 时追加 dual-real。期望 correlation、FrameApplied、DOM 与后端证据闭合，不用固定 sleep。
- **V-006 Mobile contract**: byte-exact 同步 Helix Fixture，执行链接生产 SDK/Bridge/SurfaceRuntime/QuickJS binding 的 GTest；期望 typed view 与 Helix Frame 字段一致，默认 production build 不可达测试注入和 raw reducer。
- **V-007 Cross-repo review**: 执行行为、代码边界、Frame V2 与 evidence review；期望 `G-16c` 为 `review-green`，否则保持 partial/blocked。

## Evidence Rules

- plan 阶段必须把每个 V-xxx 解析为当前仓真实存在的命令、测试文件、fixture 目录、report 路径和 timeout；无法定位时不得用猜测命令补位。
- 每次运行保留原始输出并通过 `loop_engine.py record-evidence` 登记；失败输出不得被说明文字替换。
- Mock、AcceptedCommand、HTTP 2xx、单端 replay 或 GTest 不能代替 PC Real 与跨仓 Review。
- 当前 Feature 只定义验收合同；在实现与运行证据出现前，本 Gate 状态保持 Draft。

## Implementation Evidence

### Source Pins

- Helix acceptance：`a6f5bacadeb2c13944f292d46e58dd441d27ffbe`；Fixture
  producer：`32740014cf30ed92998ea89a504597fbaef6e7b5`。
- cses-client：`48c48b04e6eb980334bade119fb34b859b1e435a`，四个 Helix
  production crate 均 pin `a6f5baca`。
- Mobile QuickJS：`1b0b11d33193a80c362491127a9d8d1e5227008c`，Helix pin
  `a6f5baca`，bundle SHA-256
  `7870e1d06d8927a52938dcd82f7ed2fbf8356e320e01b05ad3aca700fdb5c939`。
- Go：`870200d2931494da7e3fc3ac3a47e80e6646ebe4`；Strict Matrix
  SHA-256：`0e45a64ee1629498f44788613fb142489bfffbf7d02d950e8a51ae06afb419cd`。

### V-001..V-003 Helix And Fixture

- `cargo check -p helix-im`：exit 0。
- `g16c_full_flow_test`：`6/6`；`g16c_wdio_fixture_contract_test`：PASS。
- `channel-summary-active/info/display-name/notice/observer/recovery` 六份 Frame 与
  provenance 在 Helix、PC、Mobile byte-exact；provenance SHA-256：
  `9647b0b8c1b399869d238e5d7ee021f8abfd710269c86a22334621b0af437ea2`。

### V-004..V-005 PC And Go

- focused Jest/Fixture：`34/34`；G16c Mock：`3/3`。
- candidate build：Angular WDIO frontend 与 Tauri debug candidate exit 0。
- single-real：`single-real-20260727031737283-72296`，`1/1`；fresh group
  经真实 UI/Tauri/Helix/HTTP、PostgreSQL authority、WS、FrameApplied 到 DOM。
- dual-real：`dual-real-20260727032419543-78339`，`1/1`；管理员 Actor 修改
  displayName，Observer 经独立 Tauri/WS/FrameApplied/DOM 收敛。
- Go `go test ./internal/app ./internal/store/sqlstore`、`go build ./...`、
  `go vet ./...` 全通过；真实 server 使用隔离 `18066` 与同步发布 fallback，
  因此本证据覆盖真实 Go/PG/WS，但不声明 Pulsar broker plane。
- report：`1/1`；production boundary：`checkedFiles=2735`、
  `mockProductionReachable=false`。`tauri-driver not found` 为 embedded provider
  的非阻断诊断，最终各 required report exit 0。

### V-006 Mobile

- G16c Fixture parity：`6/6`；source pin、Command API check 与 legacy
  hard-cut 通过。
- `G16CFrameContractTest.*`：`5/5`；覆盖完整字段、patch/recovery 原子 apply、
  corrupt/stale fail-closed、QuickJS typed view 与 production C ABI。
- `Uc16ChannelLifecycleTest.CanonicalSettingsFrameBatchReachesTypedSummary`：
  `1/1`。旧 raw settings characterization 已由 canonical Frame batch 替换。

### V-007 Review

- **Behavior Green**：管理员权限、persist barrier、重复 authority、recovery、
  fresh group 与 Actor/Observer 最终可见结果闭合；非管理员写入不作为成功场景。
- **Code Boundary Green**：设置字段、capability、revision 与 mutation 只由
  Helix/Go authority 生产；PC/Mobile 无业务 reducer、raw fallback 或逐 mutation callback。
- **Frame V2 Green**：schema/runtime/scope/epoch/revision/baseRevision、
  snapshot/patch、settings action、directive 与 correlation 均由 producer Fixture 固定。
- **Evidence Green**：required pass rate=`1.0`、failed=`0`、skipped=`0`；
  Pulsar 未纳入本次 claim，汇总见
  [evidence/gate-report.json](evidence/gate-report.json)。
