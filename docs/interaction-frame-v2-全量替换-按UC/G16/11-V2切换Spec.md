# G16/G16b/G16c V2-only 切换 Spec

**Gate**: G16、G16b、G16c | **父规格**: `018-interaction-frame-v2-cutover` | **状态**: Draft

**执行**: [Plan](12-V2切换Plan.md) → [Tasks](13-V2切换Tasks.md)

## 目标与用户场景

管理员或成员关闭频道、退出、移除成员或修改治理属性后，按 viewer 生成不同终局：关闭只读、自退群离开页面、移除只影响目标成员、设置按版本刷新。

## 合同与影响面

| 维度 | G16 合同 |
|---|---|
| 输入/命令 | close/leave/settings binding → close/leave/change-info commands |
| Effect/Persist | http_write、storage_upsert、projection、broadcast；`channel`、`channel_member` |
| Go | `/channel/close` 及 member/settings API；`channel_close` 和治理事件 |
| PG | `channel`、`channel_member`、`channel_event` |
| SQLite | `channel`、`channel_member` |
| Surface/VM | channel-summary、left-dialog-list、composer、member-roster |
| Angular | group-settings-drawer、message-ui-header、surface runtime view |

## 专属证据清单

- [ ] close、自退群、remove member、role/settings change 分别有 binding、wire 和终局证据。
- [ ] PG lifecycle/membership/settingsVersion 与各 viewer SQLite 状态对账。
- [ ] close 禁用 composer；self leave detach；remove 只 detach 目标 viewer；其他成员继续可用。
- [ ] version conflict、无权限、重复 terminal、late event 和 PersistErr 保留正确旧状态。
- [ ] actor/被移除者/普通 observer 的 DOM、directive 和截图分别验证。

## 验收标准

五类治理动作不混用语义，viewer-specific 终局准确；纯 V2 两轮通过。

## 执行入口

工具与公共证据面见 [`../09-逐Gate-Spec公共验收合同.md`](../09-逐Gate-Spec公共验收合同.md)；字段细节见本目录 `06`-`10`。
