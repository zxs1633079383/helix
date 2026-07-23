# G06 V2-only 切换 Spec

**Gate**: G06 | **父规格**: `018-interaction-frame-v2-cutover` | **状态**: Draft

## 目标与用户场景

用户提交 reaction/快捷回复后，以服务端权威聚合更新时间线与线程；Angular 不自行累加 emoji 计数。

## 合同与影响面

| 维度 | G06 合同 |
|---|---|
| 输入/命令 | `ActionBinding(react)` → `im_quick_reply` |
| Effect/Persist | http_write、storage_upsert、projection；`message` |
| Go | `/posts/quickReply`；WS `post_update` |
| PG | `post`、`channel_event` |
| SQLite | `message` |
| Surface/VM | timeline、thread-detail；权威 reaction 聚合 VM |
| Angular | message-ui-surface-quick-actions、message-ui-action-menu |

## 专属证据清单

- [ ] actionRef 绑定 post/scope/actor，客户端不能伪造 actor。
- [ ] PG reaction 权威值、SQLite message 和两个 Surface 聚合一致。
- [ ] 重复点击、撤销/切换、并发用户、拒绝和 PersistErr 不产生错误计数。
- [ ] timeline 与 thread-detail 同一 Frame 或一致 revision 更新。
- [ ] 选中态、数量、禁用/失败态 DOM 与 observer 截图正确。

## 验收标准

reaction 聚合与 viewer 选中态准确且无端侧累加；纯 V2 两轮通过。

## 执行入口

工具与公共证据面见 [`../09-逐Gate-Spec公共验收合同.md`](../09-逐Gate-Spec公共验收合同.md)；字段细节见本目录 `06`-`10`。
