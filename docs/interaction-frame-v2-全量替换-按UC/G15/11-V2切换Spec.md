# G15/G15a V2-only 切换 Spec

**Gate**: G15、G15a | **父规格**: `018-interaction-frame-v2-cutover` | **状态**: Draft

## 目标与用户场景

创建者建立话题或新群聊后，权威频道、成员和首条内容持久化，再原子建立首批 Surface；新成员按 viewer 权限看到新频道。

## 合同与影响面

| 维度 | G15 合同 |
|---|---|
| 输入/命令 | make_topic/create_channel binding → 两个 IM command |
| Effect/Persist | http_write、storage_upsert、projection；`channel`、`message`、`reply_tree` |
| Go | `/posts/makeTopic` 及 create channel；`channel_created`、`post_update` |
| PG | `channel`、`channel_member`、`channel_topic`、`post`、`channel_event` |
| SQLite | `channel`、`channel_member`、`message`、`reply_tree` |
| Surface/VM | topic-list、channel-summary、left-dialog-list、member-roster |
| Angular | message-ui-create-topic、message-ui-create-chat、surface runtime view |

## 专属证据清单

- [ ] G15 topic 与 G15a create-channel 分别证明 wire、authority 和 Surface。
- [ ] PG channel/member/topic/post/event 与 SQLite write-set 使用新 channelId 原子对账。
- [ ] actor/observer membership、角色和首批 Surface viewer 隔离正确。
- [ ] duplicate request、部分成员无效、HTTP 成功但 WS/持久化失败均不关闭为完成。
- [ ] 新频道/话题只出现一次，列表、头部、成员和 topic 状态一致。

## 验收标准

新实体唯一、成员正确、首批 Surface 原子可用；纯 V2 两轮通过。

## 执行入口

工具与公共证据面见 [`../09-逐Gate-Spec公共验收合同.md`](../09-逐Gate-Spec公共验收合同.md)；字段细节见本目录 `06`-`10`。
