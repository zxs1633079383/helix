# G-S4 三仓验收合同

## Producer

- 唯一 producer：`crates/helix-im/src/increment_hydration.rs` 经 `ImModule::handle -> Effect::Emit`。
- Commands：im_channel_load_increment_by_channel_id。
- Persistence：channel、channel_member、message。
- Surfaces：message.channel-summary。
- Golden：`crates/helix-im/tests/fixtures/gs4-wdio/`，普通测试只比较，显式 update 才覆盖。

## PC Consumer

- Mock 只接管 command receipt；完整 Frame 从真实 `im:__bus__` replay。
- Real 必须经过 MessageV4 UI、Tauri invoke、Helix、必要 SQLite/HTTP/WS、FrameApplied 与 DOM。
- 报告必须分开 Mock/Real，记录 correlation、assertion、duration、retry、blocker 与 artifact hash。

## Mobile Consumer

- Fixture mirror 与 Helix byte-exact。
- GTest 链接生产 MobileImSdk、MobileFfiBridge、SurfaceRuntime 与 QuickJS binding。
- JS 只读 `im.surface.view` typed view；禁止 `im.store`、raw reducer 和测试业务字段。

## Physical Cold-start

- PC 444/678 与 Mobile userA/userB 必须在 production unknown-channel seed 后完全停止各自 runtime。
- 仅删除对应 persona 的 Helix SQLite main/WAL/SHM/journal，拒绝链接、活动锁、路径越界与删除后残留；不得删除 PostgreSQL 或其他本地资产。
- 每个 persona 必须在同一 DB 路径重启并执行 production unknown-channel hydration。
- Gate-scoped message/cursor/authority rows 的 remote/local hash 必须相等，missing/extra/mismatch 必须全部为 0，且 FrameApplied/typed view 同时 Green。

## Green Formula

`Helix Green + Fixture Green + PC Mock Green + PC Real Green + Mobile Contract Green + production-boundary Green + Review Green`

任一 required fail/skip/blocker 均不得 Green。
