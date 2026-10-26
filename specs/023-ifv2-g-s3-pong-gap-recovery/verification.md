# Verification Procedure: G-S3

## Required Runs

- **V-001 Helix build**: plan 必须锁定受影响 crate 并执行对应 `cargo check`；期望退出码 0，且 helix-core 运行时依赖检查仍为空。
- **V-002 Helix scenario**: 执行本 Gate 的生产 `ImModule::handle` 回放测试；期望 prerequisites、happy、failure、race/idempotency 与 recovery 的 required case 全通过。
- **V-003 Canonical Fixture**: 普通测试只生成 actual Frame 并与 checked-in JSON 比较；期望 typed validation、字段断言、raw/semantic hash 与 surface inventory 全一致，未设置显式 update 开关时不得改写 Golden。
- **V-004 PC Mock**: 执行本 Gate 的 Fixture parity、Tauri Mock replay、SurfaceRuntime、DOM 断言与 reporter；期望 required fail=0/skip=0，且报告明确标记为 Mock。
- **V-005 PC Real**: 经真实 MessageV4 UI、Tauri invoke、Helix/SQLite/HTTP/WS/bus ingress 执行 single-real；涉及 observer 时追加 dual-real。期望 correlation、FrameApplied、DOM 与后端证据闭合，不用固定 sleep。
- **V-006 Mobile contract**: byte-exact 同步 Helix Fixture，执行链接生产 SDK/Bridge/SurfaceRuntime/QuickJS binding 的 GTest；期望 typed view 与 Helix Frame 字段一致，默认 production build 不可达测试注入和 raw reducer。
- **V-007 Cross-repo review**: 执行行为、代码边界、Frame V2 与 evidence review；期望 `G-S3` 为 `review-green`，否则保持 partial/blocked。

## Evidence Rules

- plan 阶段必须把每个 V-xxx 解析为当前仓真实存在的命令、测试文件、fixture 目录、report 路径和 timeout；无法定位时不得用猜测命令补位。
- 每次运行保留原始输出并通过 `loop_engine.py record-evidence` 登记；失败输出不得被说明文字替换。
- Mock、AcceptedCommand、HTTP 2xx、单端 replay 或 GTest 不能代替 PC Real 与跨仓 Review。
- 当前 Feature 只定义验收合同；在实现与运行证据出现前，本 Gate 状态保持 Draft。

## Source Lock

- Helix: `b571b019ca677a06659e8de58bbeb8b16ae00756`
- PC: `49de0b5a4e06d15b523d61c03e28adbd155972d2`
- Mobile: `c9852e22c7eaafe260814977ccbd31dd6926f73d`
- Go: `88e754dd012aba8e966b4aaa918ea324f60988a4`
- Strict Matrix SHA-256: `caadafd2947ccadf647a134ab7c85f60b1d0feb6966f79a8717b32eef094c9c8`
- 前序 G-S2: `phase=closed`、R4 `pass`。

## Executed Evidence

| Plane | Result | Falsifiable Evidence |
|---|---|---|
| Helix | PASS | `cargo check -p helix-im`；GS3 scenario/Fixture/persist barrier/UC44 focused 共 20 tests pass |
| Fixture | PASS | Helix、PC、Mobile 四个 JSON 的逐文件 SHA-256 完全一致 |
| PC Mock | PASS | GS3 mock `2/2`；Fixture parity `3/3`；boundary `checkedFiles=2735` |
| PC Real | PASS | single seed/recover `2/2`；dual seed/recover `2/2`；444/678 hash 相等且 missing/extra/mismatch=`0/0/0` |
| PC Reset | PASS | 双 profile 共删除 main/WAL/SHM 6 个，residual=0，`remoteDeletionPerformed=false` |
| Mobile | PASS | source pin/parity/hard-cut；surface-runtime `179/179`；focused GS03 `7/7` |
| Mobile Reset | PASS | userA/userB 各删除 main/WAL/SHM 3 个，residual=0，`remoteDeletionPerformed=false` |
| Mobile Parity | PASS | userA/userB remote/local SHA-256 相等且 missing/extra/mismatch=`0/0/0` |

## Reviews

- **Behavior**: fresh group 后先持久化旧 cursor；Go 故障控制器只丢 Observer 的 post push，并以真实 pong gap 触发补偿；新 cursor 严格大于旧 cursor。
- **Code boundary**: 补偿与业务权威仍在 Helix；Go 只提供只读证据和测试故障控制；PC/Mobile 不新增业务 reducer。
- **Frame V2**: typed dialog/timeline 在 gap 恢复及删库重启后均包含相同 channel/post/marker；legacy raw event 被 typed runtime 消费，不作为验收面。
- **Evidence**: PC 444/678 与 Mobile userA/userB 均从同路径空库恢复；全量 canonical message/cursor 对账而非仅计数对账。
- **No-change tasks**: T006-T008、T016、T021、T027-T030 经生产调用链复核无需修改，既有 shared handler/bridge/reporter 已满足合同；对应 focused/runtime/boundary tests 已覆盖。

机器证据见 [gs3-four-repo-verification.json](evidence/gs3-four-repo-verification.json) 与 [gate-report.json](evidence/gate-report.json)。
