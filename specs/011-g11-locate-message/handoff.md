# G11 Handoff

## 状态

`PARTIAL - WAITING_SHARED_QUERY_COMPOSITION_AND_PERSIST_CHAIN`

## 已完成

- `normalize_located_window` 校验 exact target 与单一 channel，跨频道/target missing 整批拒绝。
- Timeline projector 拒绝 `targetMessageId` 不在 bounded complete VM 的 locate snapshot。
- `reveal_directive` 固定只有 `directiveId/kind`，不携 target payload。
- Angular 仅从 committed `TimelineWindowVm.targetMessageId` 标记唯一 `data-message-target=true`；focused 测试通过。
- 新增 `g11_locate_interaction_test.rs`。

## 证据

- Angular `message-surface-runtime-view` suite `9/9 PASS`；合并 batch wall-clock `25.33s`。
- Rust aggregate 待 root 统一执行。

## 待 root 收敛

- shared action/correlation 组合 `posts/get + postContext (+ after-index)`，并在 PersistOk 后合成 locate Frame。
- reveal 只允许 whole-frame apply/ack 后执行；not-found 不发 directive。
- 最终 DOM reveal/highlight、双库与真 Go 证据归 017。
