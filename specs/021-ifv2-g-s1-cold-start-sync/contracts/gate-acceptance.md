# G-S1 三仓验收合同

## Producer

- 唯一 producer：`crates/helix-im/src/sync/increment.rs` 经 `ImModule::handle -> Effect::Emit`。
- Commands：im_query_dialog_list。
- Persistence：channel、channel_member、channel_event_cursor。
- Surfaces：message.left-dialog-list。
- Golden：`crates/helix-im/tests/fixtures/gs1-wdio/`，普通测试只比较，显式 update 才覆盖。

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

## Physical Cold-Start Parity

- PC 使用真实 444/678 persona，Mobile 使用真实 userA/userB persona；每个 persona 在 seed 与 recover 阶段必须复用同一 Helix SQLite 路径。
- reset 只允许在客户端完全停止后删除 SQLite main、`-wal`、`-shm`、`-journal`，并必须拒绝符号链接、仍被占用的文件、目录级删除与未登记 tenant。
- 远端 PostgreSQL、认证文件、ViewModelStore、其他 tenant 和非 Helix 本地资产均禁止删除。
- 对账范围是本次 isolated Gate dataset 对当前 persona 可见的全部可映射权威行；使用稳定主键、字段 allowlist 和排序后的规范化 JSON 计算 hash。
- 报告必须分别列出 PC Actor、PC Observer、Mobile userA、Mobile userB 的 DB 路径复用、删除清单、零残留、remote/local hash、missing、extra、mismatch、FrameApplied 与可见投影。
- 任一 persona 缺失、抽样对账、只比较数量/marker、只证明 HTTP/WS/Frame/DOM，或 `missing|extra|mismatch > 0`，本 Gate 均不得 Green。
