# G05 V2-only 切换 Spec

**Gate**: G05 | **父规格**: `018-interaction-frame-v2-cutover` | **状态**: Draft

**执行**: [Plan](12-V2切换Plan.md) → [Tasks](13-V2切换Tasks.md)

## 目标与用户场景

转发者选择多个目标频道后，每个目标独立形成权威结果；一个目标失败不得被其他目标成功遮蔽。

## 合同与影响面

| 维度 | G05 合同 |
|---|---|
| 输入/命令 | `ActionBinding(forward)` → `im_relay_messages` |
| Effect/Persist | storage_read、http_write、projection；`message`、`channel` |
| Go | `/posts/createPosts`；每个目标频道 WS `post` |
| PG | `post`、`channel`、`channel_member`、`channel_event` |
| SQLite | `message`、`channel` |
| Surface/VM | 源/目标 timeline、left-dialog-list、composer |
| Angular | message-ui-forward-dialog、forward-detail、surface runtime view |

## 专属证据清单

- [ ] 每个目标有独立 correlation、temporary/canonical identity 和终态。
- [ ] PG 与 SQLite 按 target channelId + postId 分别对账，不能只查总数。
- [ ] matching WS + PersistOk 前 overlay 不报告业务完成。
- [ ] 部分成功、全失败、重复目标、无权限目标和 late echo 均可证伪。
- [ ] actor overlay 与各目标 observer 时间线/会话预览分别截图。

## 验收标准

多目标结果逐目标准确，部分失败不误报；无 V1/raw reducer，纯 V2 两轮通过。

## 执行入口

工具与公共证据面见 [`../09-逐Gate-Spec公共验收合同.md`](../09-逐Gate-Spec公共验收合同.md)；字段细节见本目录 `06`-`10`。
