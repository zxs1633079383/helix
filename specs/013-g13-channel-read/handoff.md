# G13 Implementation Handoff

## 状态

`PARTIAL - awaiting shared ActionDispatch/PersistOk/Frame integration`

## 已完成

- 新增 O(1) `ChannelReadObservationTracker`：同 epoch/window 连续 observation 才产单调 boundary intent；重复、回退和不可见 observation 不重复推进。
- Angular `message-read-reporting` 只搬运 HostObservation，不计算 unread、viewer 或业务成功。
- 新增 focused Rust/Angular tests；Rust 首轮按预期因 tracker 缺失红灯。

## 证据

- Angular：`evidence/angular-focused.log`，1/1 PASS（与 G14/G16 合批共 5/5）。
- Rust：`evidence/rust-focused.log` 的最终调用被并发 shared `module/action.rs` 编译错误阻断；shared owner 已修，需 root 聚合重跑。

## 未决

- shared `markChannelRead/observeChannelViewport -> im_channels_view` ActionDispatch、corr/authority、PersistAtomic 与 dialog/unread/timeline terminal Frame。
- Native/PG/SQLite/Frame/DOM/visual 只由 017 最终 candidate 验收。
