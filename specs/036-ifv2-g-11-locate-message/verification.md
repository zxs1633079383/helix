# Verification Procedure: G-11

## Source Lock

| Plane | Locked value | Evidence |
|---|---|---|
| Helix HEAD | `8c732a84cf0c0d29e65646e3aa5bfd7e87675856` | `git rev-parse HEAD` |
| cses-client HEAD | `f1c4ddc0fbbd4bb4547108627af83a028284a343` | `git -C .../cses-client rev-parse HEAD` |
| Mobile HEAD | `26c7616d805f0ba21406f0f9fa3579e212bdd090` | `git -C .../mobile-qucik-c++ rev-parse HEAD` |
| PC Helix pin | `d2b1bf6c1304155d6897a7238cfaefd2bae3d176` | `src-tauri/Cargo.toml` |
| Mobile Helix pin | `7e93be302d251fd4e63ca47c637dc0b3f289e1a1` | `contract/helix-source.lock.json` |
| Strict Matrix SHA-256 | `b6625b033db6ada5a05ae9d7898f664df25cd097a5a0e18d42aaa3d3e00a02ac` | current Helix contract |
| G10a prerequisite | `phase=closed,event_seq=17` | `specs/035-ifv2-g-10a-load-older/review-ledger.json` |

`check-prerequisites.sh` 的持久 feature 指针原停在 G09；本轮已显式锁到
`specs/036-ifv2-g-11-locate-message`。before-implement 在已经 admitted 后重复 advance，使 ledger
提前进入 `verifying/missing evidence`；这不作为完成证据，必须由本轮真实 evidence 修复。

## Required Runs

- **V-001 Helix build**: plan 必须锁定受影响 crate 并执行对应 `cargo check`；期望退出码 0，且 helix-core 运行时依赖检查仍为空。
- **V-002 Helix scenario**: 执行本 Gate 的生产 `ImModule::handle` 回放测试；期望 prerequisites、happy、failure、race/idempotency 与 recovery 的 required case 全通过。
- **V-003 Canonical Fixture**: 普通测试只生成 actual Frame 并与 checked-in JSON 比较；期望 typed validation、字段断言、raw/semantic hash 与 surface inventory 全一致，未设置显式 update 开关时不得改写 Golden。
- **V-004 PC Mock**: 执行本 Gate 的 Fixture parity、Tauri Mock replay、SurfaceRuntime、DOM 断言与 reporter；期望 required fail=0/skip=0，且报告明确标记为 Mock。
- **V-005 PC Real**: 经真实 MessageV4 UI、Tauri invoke、Helix/SQLite/HTTP/WS/bus ingress 执行 single-real；涉及 observer 时追加 dual-real。期望 correlation、FrameApplied、DOM 与后端证据闭合，不用固定 sleep。
- **V-006 Mobile contract**: byte-exact 同步 Helix Fixture，执行链接生产 SDK/Bridge/SurfaceRuntime/QuickJS binding 的 GTest；期望 typed view 与 Helix Frame 字段一致，默认 production build 不可达测试注入和 raw reducer。
- **V-007 Cross-repo review**: 执行行为、代码边界、Frame V2 与 evidence review；期望 `G-11` 为 `review-green`，否则保持 partial/blocked。

## Evidence Rules

- plan 阶段必须把每个 V-xxx 解析为当前仓真实存在的命令、测试文件、fixture 目录、report 路径和 timeout；无法定位时不得用猜测命令补位。
- 每次运行保留原始输出并通过 `loop_engine.py record-evidence` 登记；失败输出不得被说明文字替换。
- Mock、AcceptedCommand、HTTP 2xx、单端 replay 或 GTest 不能代替 PC Real 与跨仓 Review。
- 当前 Feature 只定义验收合同；在实现与运行证据出现前，本 Gate 状态保持 Draft。

## Helix Producer Run

- `cargo check -p helix-im`: exit `0`, `6.51s`。
- `cargo test -p helix-im --test g11_full_flow_test --test g11_locate_interaction_test --test read_relay_test -- --nocapture`:
  exit `0`；G11 full flow `7/7`、字段 helper `1/1`、read relay regression `11/11`。
- 生产结果：`posts/get` body 仅含 `postIds`；HTTP reply 必须与 attached target/channel 同一；
  success 只发布一个 `message.timeline` Frame，`targetMessageId`、`anchor.mode=locate` 与
  payload-free `revealMessage` directive 同 Frame；transport failure、absent target 与 duplicate reply
  均产生 `0` Effect。
