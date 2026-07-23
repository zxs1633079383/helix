# G05 V2-only 切换 Plan

**Spec**: [11-V2切换Spec.md](11-V2切换Spec.md) | **聚合计划**: [`018/plan.md`](../../../specs/018-interaction-frame-v2-cutover/plan.md)

## 目标

多目标转发以 target-scoped correlation 独立收敛，部分失败不被批量 HTTP 接受或其他目标成功覆盖。

## 影响文件

- Helix：`src/forward.rs`、render-ready forward、`tests/g05_forward_interaction_test.rs`
- cses-client：forward dialog/detail、timeline/runtime 与 G05 tests
- 数据：PG `post/channel/channel_member/channel_event`；SQLite `message/channel`

## 实施顺序

RED 覆盖重复目标、部分失败、无权限、late echo；实现逐目标 authority/persist/V2 result；客户端保持 overlay 到终局；按 target channel/post 查询证据。

## 完成闸门

每个目标独立 PASS/FAIL，observer 结果正确，无 V1/raw fallback。
