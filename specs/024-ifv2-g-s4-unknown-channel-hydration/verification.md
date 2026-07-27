# Verification Procedure: G-S4

## Required Runs

- **V-001 Helix build**: plan 必须锁定受影响 crate 并执行对应 `cargo check`；期望退出码 0，且 helix-core 运行时依赖检查仍为空。
- **V-002 Helix scenario**: 执行本 Gate 的生产 `ImModule::handle` 回放测试；期望 prerequisites、happy、failure、race/idempotency 与 recovery 的 required case 全通过。
- **V-003 Canonical Fixture**: 普通测试只生成 actual Frame 并与 checked-in JSON 比较；期望 typed validation、字段断言、raw/semantic hash 与 surface inventory 全一致，未设置显式 update 开关时不得改写 Golden。
- **V-004 PC Mock**: 执行本 Gate 的 Fixture parity、Tauri Mock replay、SurfaceRuntime、DOM 断言与 reporter；期望 required fail=0/skip=0，且报告明确标记为 Mock。
- **V-005 PC Real**: 经真实 MessageV4 UI、Tauri invoke、Helix/SQLite/HTTP/WS/bus ingress 执行 single-real；涉及 observer 时追加 dual-real。期望 correlation、FrameApplied、DOM 与后端证据闭合，不用固定 sleep。
- **V-006 Mobile contract**: byte-exact 同步 Helix Fixture，执行链接生产 SDK/Bridge/SurfaceRuntime/QuickJS binding 的 GTest；期望 typed view 与 Helix Frame 字段一致，默认 production build 不可达测试注入和 raw reducer。
- **V-007 Cross-repo review**: 执行行为、代码边界、Frame V2 与 evidence review；期望 `G-S4` 为 `review-green`，否则保持 partial/blocked。

## Evidence Rules

- plan 阶段必须把每个 V-xxx 解析为当前仓真实存在的命令、测试文件、fixture 目录、report 路径和 timeout；无法定位时不得用猜测命令补位。
- 每次运行保留原始输出并通过 `loop_engine.py record-evidence` 登记；失败输出不得被说明文字替换。
- Mock、AcceptedCommand、HTTP 2xx、单端 replay 或 GTest 不能代替 PC Real 与跨仓 Review。
- 当前 Feature 只定义验收合同；在实现与运行证据出现前，本 Gate 状态保持 Draft。

## Source Lock

- Helix: `ef9494262fc4fdc42d355442d3866236c6a4e4de`
- PC: `ed538609be097e583af4b05228303b5c522ec546`
- Mobile: `084fc8392004398eb162fef528f8b59ffdc5813f`
- Go: `e4815b7e57e0ed0f54a1b0ebad1b4bfbf2e2880d`
- Strict Matrix SHA-256: `caadafd2947ccadf647a134ab7c85f60b1d0feb6966f79a8717b32eef094c9c8`
- 前序 G-S3: `phase=closed`、R1-R4 PASS。

## Executed Evidence

| Plane | Result | Falsifiable Evidence |
|---|---|---|
| Helix | PASS | `cargo check -p helix-im`；`gs4_full_flow_test` 与 `gs4_wdio_fixture_contract_test` 共 `7/7` PASS |
| Fixture | PASS | Helix、PC、Mobile 的 3 个 Frame 与 provenance 逐文件 SHA-256 完全一致 |
| PC Mock | PASS | GS4 Mock `2/2`；Fixture parity、bridge、report 与 production boundary 均 PASS |
| PC Real | PASS | single-real 与 dual-real 均 PASS；444/678 canonical message/cursor/authority hash 相等，missing/extra/mismatch=`0/0/0` |
| PC Reset | PASS | single profile 删除 main/WAL/SHM 3 个；dual profile 删除 6 个；residual=`0`，`remoteDeletionPerformed=false` |
| Go Evidence | PASS | `go test ./cmd/server-db-evidence` PASS；新增 `gs4-parity` 只读权威对账入口，无远端写操作 |
| Mobile | PASS | source pin、Fixture parity、legacy hard-cut；surface-runtime `182/182`；focused GS04 `7/7` |
| Mobile Reset | PASS | userA/userB 各删除 main/WAL/SHM 3 个；`residualArtifacts=[]`，`remoteDeletionPerformed=false` |
| Mobile Parity | PASS | userA/userB remote/local SHA-256 均为 `f2cc1762783a5feb41e74934811d0a270d02f374460fc7aea9c4ba05adbad18b`，missing/extra/mismatch=`0/0/0` |

## Reviews

- **Behavior**: fresh group 由 userA 创建并写入三条消息；userB 经生产 `ensureLoaded` 完成陌生频道补水；非成员频道 fail-closed。四 persona 删库后均由同一 DB 路径重启并恢复 typed summary、timeline 与 dialog。
- **Code boundary**: 权限、补水、sync 与投影权威仍在 Helix。Go 仅增加只读证据操作；PC/Mobile 仅编排真实流程、机械消费 Frame 与采集证据，没有新增业务 reducer。
- **Frame V2**: `dialog-empty`、`dialog-synced`、`dialog-recovery` 的 envelope、scope、epoch、revision、mutation、action、directive 与 raw/semantic hash 在三仓一致。
- **Evidence**: PC 444/678 与 Mobile userA/userB 都物理删除本地 Helix SQLite 后再恢复；对账覆盖完整 message/cursor/authority 集合，不以计数、HTTP 200 或即时 `im:read:result` 代替持久终态。
- **Runtime drift resolved**: Mobile consumer pin 更新后必须显式重建 `helix-driver-ffi`；旧动态库曾导致 `updateAt` 对账为 0。按 lock 重建后双 persona 全量差异归零。
- **No-change tasks**: T006-T008、T016、T021、T027-T030 经生产调用链复核无需修改；既有 handler、bridge、runtime 与 binding 已满足合同，聚焦测试和边界扫描覆盖其行为。

机器证据见 [gs4-four-repo-verification.json](evidence/gs4-four-repo-verification.json) 与 [gate-report.json](evidence/gate-report.json)。
