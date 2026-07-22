# G10 Handoff

## 状态

`PARTIAL - WAITING_SHARED_ACTION_AND_PERSIST_CHAIN`

## 已完成

- 新增 focused Rust 合同 `g10_timeline_window_interaction_test.rs`，固定 patch 为完整 `replaceView`、windowToken 稳定及 older/newer page truth。
- Timeline projector 保持有界完整 VM；Angular runtime 已有 page/token/anchor data contract，focused suite 通过。

## 证据

- Angular `message-surface-runtime-view` suite `9/9 PASS`，包含 committed target marker；合并 batch wall-clock `25.33s`。
- Rust aggregate 待 root 统一执行。

## 待 root 收敛

- `AttachTimeline` 必须是 HostObservation；`loadOlder/loadNewer` 才是 ActionBinding。
- local/HTTP authority 必须 PersistAtomic coverage/index 后才发布窗口 Frame；shared action/correlation/persist owner 未由本 agent 修改。
- 首屏/补窗真实滚动与视觉证据归 017。
