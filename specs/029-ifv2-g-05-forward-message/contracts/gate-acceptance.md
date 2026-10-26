# G-05 三仓验收合同

## Producer

- 唯一 producer：`crates/helix-im/src/forward.rs` 经 `ImModule::handle -> Effect::Emit`。
- Commands：im_relay_messages。
- Persistence：message、channel。
- Surfaces：message.timeline。
- Golden：`crates/helix-im/tests/fixtures/g05-wdio/`，普通测试只比较，显式 update 才覆盖。

## PC Consumer

- Mock 只接管 command receipt；完整 Frame 从真实 `im:__bus__` replay。
- Real 必须经过 MessageV4 UI、Tauri invoke、Helix、必要 SQLite/HTTP/WS、FrameApplied 与 DOM。
- 报告必须分开 Mock/Real，记录 correlation、assertion、duration、retry、blocker 与 artifact hash。

## Mobile Consumer

- Fixture mirror 与 Helix byte-exact。
- GTest 链接生产 MobileImSdk、MobileFfiBridge、SurfaceRuntime 与 QuickJS binding。
- JS 只读 `im.surface.view` typed view；禁止 `im.store`、raw reducer 和测试业务字段。

## Green Formula

`Helix Green + Fixture Green + PC Mock Green + PC Real Green + Mobile Contract Green + production-boundary Green + Review Green`

任一 required fail/skip/blocker 均不得 Green。
