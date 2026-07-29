# Verification Procedure: G-16b

## Source Lock

- Helix acceptance HEAD：`71d1350b330449ddb1d61b48a854b31c7640f18f`；前序
  G-16 已由 `4630aaa` 收口，`review-ledger.json` 为 `phase=closed`、
  `r4_evidence=pass`。
- cses-client HEAD：`aac3c1583519b9308acb7c8dd0e415fa1dfc3813`；Mobile HEAD：
  `46455d4d4e88870b75e72fdf1caa9f518eb87835`。
- Strict Matrix SHA-256：
  `0e45a64ee1629498f44788613fb142489bfffbf7d02d950e8a51ae06afb419cd`。
- 当前 Gate 只处理 WS `channel_member_update.memberChange.leave` 对
  `channel_member` durable fact 与 `message.member-roster` 的广播投影；G16 已完成的
  `im_channel_leave -> message.channel-summary lifecycle=left` 不在本 Gate 重复实现。

## Required Runs

- **V-001 Helix build**: plan 必须锁定受影响 crate 并执行对应 `cargo check`；期望退出码 0，且 helix-core 运行时依赖检查仍为空。
- **V-002 Helix scenario**: 执行本 Gate 的生产 `ImModule::handle` 回放测试；期望 prerequisites、happy、failure、race/idempotency 与 recovery 的 required case 全通过。
- **V-003 Canonical Fixture**: 普通测试只生成 actual Frame 并与 checked-in JSON 比较；期望 typed validation、字段断言、raw/semantic hash 与 surface inventory 全一致，未设置显式 update 开关时不得改写 Golden。
- **V-004 PC Mock**: 执行本 Gate 的 Fixture parity、Tauri Mock replay、SurfaceRuntime、DOM 断言与 reporter；期望 required fail=0/skip=0，且报告明确标记为 Mock。
- **V-005 PC Real**: 经真实 MessageV4 UI、Tauri invoke、Helix/SQLite/HTTP/WS/bus ingress 执行 single-real；涉及 observer 时追加 dual-real。期望 correlation、FrameApplied、DOM 与后端证据闭合，不用固定 sleep。
- **V-006 Mobile contract**: byte-exact 同步 Helix Fixture，执行链接生产 SDK/Bridge/SurfaceRuntime/QuickJS binding 的 GTest；期望 typed view 与 Helix Frame 字段一致，默认 production build 不可达测试注入和 raw reducer。
- **V-007 Cross-repo review**: 执行行为、代码边界、Frame V2 与 evidence review；期望 `G-16b` 为 `review-green`，否则保持 partial/blocked。

## Evidence Rules

- plan 阶段必须把每个 V-xxx 解析为当前仓真实存在的命令、测试文件、fixture 目录、report 路径和 timeout；无法定位时不得用猜测命令补位。
- 每次运行保留原始输出并通过 `loop_engine.py record-evidence` 登记；失败输出不得被说明文字替换。
- Mock、AcceptedCommand、HTTP 2xx、单端 replay 或 GTest 不能代替 PC Real 与跨仓 Review。
- 当前 Feature 只定义验收合同；在实现与运行证据出现前，本 Gate 状态保持 Draft。

## Implementation Evidence

### Source Pins

- Helix acceptance：`74ebe9a6ba90c7506abc091f541521f6cd33c8cf`；canonical
  Fixture producer：`8c9a9706cb865d0849b17b91485f2e6f2480a4bd`。
- cses-client：`d89722dbb5d41b9944a3df2d4a32f354fdf24ea5`，其四个
  Helix crates 均 pin `74ebe9a6`。
- Mobile QuickJS：`164fd3d95b015d836d715fea1815f29c411d2dea`，reviewed
  candidate pin `74ebe9a6`，bundle SHA-256
  `e3b1ece09f59d812b2caa32e000168035c12b4bd81b8511a284512c49fe03763`。
- Strict Matrix SHA-256 保持
  `0e45a64ee1629498f44788613fb142489bfffbf7d02d950e8a51ae06afb419cd`。

### V-001..V-003 Helix And Fixture

- `cargo check -p helix-im`：exit 0。
- `cargo test -p helix-im --test g16b_full_flow_test --test
  g16b_wdio_fixture_contract_test -- --nocapture`：`5/5 + 1/1`。
- `cargo test -p helix-im --test t081_surface_resync_producer_test --test
  g16b_full_flow_test --test g16b_wdio_fixture_contract_test -- --nocapture`：
  `4/4 + 5/5 + 1/1`。
- `cargo tree -p helix-core | grep -iE 'tokio|sqlx|reqwest|web-sys|tauri'`：
  空输出。
- 三仓 `member-roster-active/left/recovery` 与 provenance 的 SHA-256 逐文件
  相等；provenance 为
  `18677799878712f76cfae4fa0acef99491b39df4c3bcf6998d991b1f0418267e`。

### V-004..V-005 PC

- focused Jest：`3 suites / 32 tests` 全通过；G16b Mock：`3/3`。
- candidate build：Angular WDIO frontend 与 Tauri debug candidate exit 0。
- single-real：`single-real-20260727003952919-22983`，`1/1`；证明 fresh
  group、真实 Tauri/Helix/SQLite readback、FrameApplied 与 typed roster DOM。
- dual-real：`dual-real-20260727005242330-33969`，`1/1`；Observer 通过真实
  Tauri `im_channel_leave`，Actor 经真实 WS、durable roster Frame、
  FrameApplied 与 DOM 从 `444/678` 收敛为 `444`。
- report contract：`1/1`；production boundary：`checkedFiles=2735`、
  `mockProductionReachable=false`。
- 独立 `tauri-driver not found` 诊断不阻断配置内嵌 WebKit/Tauri session；
  最终 single/dual report 均 exit 0。

### V-006 Mobile

- G16b Fixture parity：`3/3`，source pin 与 Command API write/check 通过。
- `G16BFrameContractTest.*`：`6/6`；覆盖完整字段、patch 原子 apply、recovery、
  corrupt/stale fail-closed、QuickJS typed view 与 production C ABI。
- `Uc16ChannelLifecycleTest.CanonicalMemberLeaveBroadcastReachesTypedRoster`：
  `1/1`。
- legacy hard-cut：production V1 producer=`0`、consumer=`0`，G16b
  typed-only；既有 SDK/Bridge/SurfaceRuntime/QuickJS 已满足合同，因此
  T027-T030 无需修改生产文件。

### V-007 Review

- **Behavior Green**：self-left 与远端 `channel_member_update` 分离；只有
  `channel_member` PersistAtomic 成功后才刷新已 attach roster，重复 authority
  单次生效，持久失败可重放，cold attach 从 durable rows 恢复。
- **Code Boundary Green**：成员集合、membership、revision 与 mutation kind
  仅由 Helix 裁决；PC/Mobile 不新增领域 reducer、raw fallback 或逐 mutation
  callback。
- **Frame V2 Green**：schema/runtime/scope/epoch/revision/baseRevision、
  complete replace view、disabled action、directive 与 correlation 均由 producer
  Fixture 固定，三仓 byte-exact。
- **Evidence Green**：required pass rate=`1.0`、failed=`0`、skipped=`0`；
  汇总见 [evidence/gate-report.json](evidence/gate-report.json)。
