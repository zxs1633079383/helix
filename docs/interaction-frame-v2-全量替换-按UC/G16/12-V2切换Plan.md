# G16/G16b/G16c V2-only 切换 Plan

**Spec**: [11-V2切换Spec.md](11-V2切换Spec.md) | **聚合计划**: [`018/plan.md`](../../../specs/018-interaction-frame-v2-cutover/plan.md)

## 目标

close/self-leave/remove-member/role-settings 使用分离 binding 和 viewer-specific terminal Frame：关闭只读、自退群 detach、移除只影响目标、设置按 version 刷新。

## 影响文件

- Helix：channel admin/member/settings normalize/projectors、`tests/g16_governance_interaction_test.rs`
- cses-client：settings/header/member/channel-management 与 G16 tests
- 数据：PG `channel/channel_member/channel_event`；SQLite `channel/channel_member`

## 实施顺序

RED 分别覆盖五类动作、权限、version conflict、重复 terminal、late event、PersistErr；实现 viewer-specific persist/compose/directive；客户端按终局渲染；三角色证据。

## 完成闸门

五类治理语义不混用，actor/target/observer 终局正确。
