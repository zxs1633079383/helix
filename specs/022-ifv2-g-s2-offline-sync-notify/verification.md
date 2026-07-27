# Verification Procedure: G-S2

## Source Lock

- Helix worktree: `967bfec78d69922b6ecc0fb6e4b11ea2d5e73195`；
  前序 G-S1 已由 Spec Kit event 45 `CLOSED`，tasks 46/46、R1-R4 PASS。
- cses-client worktree: `fce658a81bddcc90c66145d87bf8e6aa643bbf5a`；
  Mobile worktree: `26c5abeca8d6b48256fe30f1836c9ef9d58c631f`；
  Go worktree: `e3f7c170ed681845f07cc95788c2f3a9fc799790`。
- Helix runtime source pin remains
  `33b6cf2d5621f6fc417a1229111b58bd6854d713` in PC/Mobile consumers。
- Strict Matrix SHA-256:
  `caadafd2947ccadf647a134ab7c85f60b1d0feb6966f79a8717b32eef094c9c8`。
- 本 Gate 只处理 explicit/offline sync notify；pong gap 与 unknown-channel
  hydration 分别保留给 G-S3/G-S4。

## Required Runs

- **V-001 Helix build**: plan 必须锁定受影响 crate 并执行对应 `cargo check`；期望退出码 0，且 helix-core 运行时依赖检查仍为空。
- **V-002 Helix scenario**: 执行本 Gate 的生产 `ImModule::handle` 回放测试；期望 prerequisites、happy、failure、race/idempotency 与 recovery 的 required case 全通过。
- **V-003 Canonical Fixture**: 普通测试只生成 actual Frame 并与 checked-in JSON 比较；期望 typed validation、字段断言、raw/semantic hash 与 surface inventory 全一致，未设置显式 update 开关时不得改写 Golden。
- **V-004 PC Mock**: 执行本 Gate 的 Fixture parity、Tauri Mock replay、SurfaceRuntime、DOM 断言与 reporter；期望 required fail=0/skip=0，且报告明确标记为 Mock。
- **V-005 PC Real**: 经真实 MessageV4 UI、Tauri invoke、Helix/SQLite/HTTP/WS/bus ingress 执行 single-real；涉及 observer 时追加 dual-real。期望 correlation、FrameApplied、DOM 与后端证据闭合，不用固定 sleep。
- **V-006 Mobile contract**: byte-exact 同步 Helix Fixture，执行链接生产 SDK/Bridge/SurfaceRuntime/QuickJS binding 的 GTest；期望 typed view 与 Helix Frame 字段一致，默认 production build 不可达测试注入和 raw reducer。
- **V-007 Cross-repo review**: 执行行为、代码边界、Frame V2 与 evidence review；期望 `G-S2` 为 `review-green`，否则保持 partial/blocked。

## Evidence Rules

- plan 阶段必须把每个 V-xxx 解析为当前仓真实存在的命令、测试文件、fixture 目录、report 路径和 timeout；无法定位时不得用猜测命令补位。
- 每次运行保留原始输出并通过 `loop_engine.py record-evidence` 登记；失败输出不得被说明文字替换。
- Mock、AcceptedCommand、HTTP 2xx、单端 replay 或 GTest 不能代替 PC Real 与跨仓 Review。
- 当前 Feature 只定义验收合同；在实现与运行证据出现前，本 Gate 状态保持 Draft。

## Final Source Pins

- Helix acceptance: `91b11b244fd4f36e72c044e2bb3645c1c7a2d60f`；
  GS2 production fixes are `d59f27c3` and `e95c6d74`，fixture/test commit is
  `91b11b24`。
- Fixture producer recorded by all three mirrors:
  `33b6cf2d5621f6fc417a1229111b58bd6854d713`。
- PC implementation: `37dc70a82`；runtime Helix pin:
  `e95c6d74d5383f9d18c820a57fbc3693dd55cd0e`。
- Mobile implementation: `6692abb`；source pin:
  `91b11b244fd4f36e72c044e2bb3645c1c7a2d60f`；
  bundle SHA-256:
  `0574aca6189475cc0e2189f757a18e13652d4f1eed7649f9fbf3c0d7f5964726`。
- Go read-only evidence command: `7a3865b`。
- Strict Matrix SHA-256:
  `caadafd2947ccadf647a134ab7c85f60b1d0feb6966f79a8717b32eef094c9c8`。

## Executed Evidence

| Plane | Command / Artifact | Result |
|---|---|---|
| Helix producer | `cargo test -p helix-im --test gs2_full_flow_test --test gs2_wdio_fixture_contract_test` | PASS: full flow 6/6, fixture contract 1/1 |
| Fixture parity | Helix/PC/Mobile `gs2-wdio` mirrors | PASS: 3 fixtures, schema V2, raw/semantic provenance equal |
| PC Mock | `CSES_WDIO_WEB_PORT=14201 yarn message:wdio:gs2:mock` | PASS: 2/2; rejected admission preserves business projection |
| PC single-real | `artifacts/message-wdio/gs2-cold-single-20260727051039/` | PASS: seed/reset/recover; actor hash equal, differences 0 |
| PC dual-real | `artifacts/message-wdio/dual-real-20260727123618039-35026/` | PASS: actor/observer seed/reset/recover and typed DOM |
| PC boundary | `yarn message:wdio:gs2:boundary` | PASS: 2735 production files, mockProductionReachable=false |
| Mobile | `./scripts/run-uc01-16-gtest.sh --only GS02` | PASS: 9/9; production SDK/FFI/SurfaceRuntime/QuickJS real chain |
| Go evidence | `go test ./cmd/server-db-evidence ./internal/realchain/dbevidence` | PASS |

## Four-persona Cold-start Evidence

- PC run `dual-real-20260727123618039-35026` deleted exactly six local
  SQLite artifacts: actor and observer main/WAL/SHM；journal was absent。
  `residualArtifacts=[]` and `remoteDeletionPerformed=false`。
- PC actor and observer each produced
  `remoteSha256=localSha256=4b6a668a0cd0edb0448862b34083ba2a1ae833c9ecb060f69b3c65be703156ba`；
  missing/extra/mismatch are all empty。
- Mobile run `mrc-mobile-uc-20260727132726-83862` deleted only each
  persona's Helix SQLite transaction family and reused the same database path。
  Both reset evidence files record zero residual and
  `remoteDeletionPerformed=false`。
- Mobile userA and userB each produced
  `remoteSha256=localSha256=1355e2bbfa20fcfde94898bd640ca1fa657b581fe386def3be1aaa8c3a206d88`；
  missing/extra/mismatch are all empty。
- PC recover reports contain FrameApplied/DOM dialog convergence；Mobile
  userA/userB cold and restart phases each returned
  `typedDialogFrame=true` and `typedTimelineFrame=true`。
- PostgreSQL evidence ran through a temporary CNPG port-forward with
  `default_transaction_read_only=on`；no remote deletion command exists in
  the GS2 evidence path。

## Reviews

- **Behavior Green**: fresh group、old cursor、three offline messages、
  fail-closed transport/persist、late reply idempotency、restart no duplicate、
  Actor/Observer and userA/userB final outcomes agree。
- **Code Boundary Green**: business reduction and persistence authority stay in
  Helix；PC/Mobile only admit commands, transport batches, apply typed Frames,
  and collect test-only evidence。Generic mock bridge and reporter were reused
  rather than adding GS2 production branches。
- **Frame V2 Green**: schema/surface/scope/epoch/revision、snapshot/patch、
  dialog mutations、actions/directives and correlation are byte-derived from
  the producer fixtures across Helix、PC and Mobile。
- **Evidence Green**: required pass rate 100%，failed=0，skipped=0；
  four personas have physical reset, same-path recovery, full remote/local
  equality and typed UI convergence。
