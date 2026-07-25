# InteractionFrame V2 26 Gate Spec Kit 索引

> 本索引是总控 Spec 的人类可读视图；机器真源为 [gate-spec-manifest.json](gate-spec-manifest.json)。
> 一个 Gate 完成 Helix、PC、Mobile 与 UC Review 后，才能进入下一行。

| 顺序 | Gate | UC | 业务 | Spec Kit Feature | 状态/依赖 |
|---:|---|---|---|---|---|
| 1 | G-01 | UC-1.1 | 发送文本 | [019 G01 基线](../019-g01-wdio-five-phases/spec.md) | baseline Green；最终候选重验 |
| 2 | G-02 | UC-1.2 | 发文档 | [025-ifv2-g-02-document-message](../025-ifv2-g-02-document-message/spec.md) | Draft；依赖 G-01 review-green |
| 3 | G-03a | UC-1.4 | 重发失败 | [026-ifv2-g-03a-retry-failed-send](../026-ifv2-g-03a-retry-failed-send/spec.md) | Draft；依赖 G-02 review-green |
| 4 | G-04 | UC-1.5 | 撤回 | [027-ifv2-g-04-online-revoke](../027-ifv2-g-04-online-revoke/spec.md) | Draft；依赖 G-03a review-green |
| 5 | G-04a | UC-1.5-offline | 离线撤回重放 | [028-ifv2-g-04a-offline-revoke](../028-ifv2-g-04a-offline-revoke/spec.md) | Draft；依赖 G-04 review-green |
| 6 | G-05 | UC-1.7 | 转发/合并转发 | [029-ifv2-g-05-forward-message](../029-ifv2-g-05-forward-message/spec.md) | Draft；依赖 G-04a review-green |
| 7 | G-06 | UC-1.8 | 快捷回复 emoji | [030-ifv2-g-06-quick-reaction](../030-ifv2-g-06-quick-reaction/spec.md) | Draft；依赖 G-05 review-green |
| 8 | G-07 | UC-1.9 | 加急 + 确认 | [031-ifv2-g-07-urgent-message](../031-ifv2-g-07-urgent-message/spec.md) | Draft；依赖 G-06 review-green |
| 9 | G-08 | UC-1.10 | 定时消息 | [032-ifv2-g-08-schedule-create](../032-ifv2-g-08-schedule-create/spec.md) | Draft；依赖 G-07 review-green |
| 10 | G-09 | UC-1.10-cancel | 取消定时 | [033-ifv2-g-09-schedule-cancel](../033-ifv2-g-09-schedule-cancel/spec.md) | Draft；依赖 G-08 review-green |
| 11 | G-10 | UC-2.1 | 切群首屏 | [034-ifv2-g-10-timeline-window](../034-ifv2-g-10-timeline-window/spec.md) | Draft；依赖 G-09 review-green |
| 12 | G-10a | UC-2.2 | 上拉更早历史 | [035-ifv2-g-10a-load-older](../035-ifv2-g-10a-load-older/spec.md) | Draft；依赖 G-10 review-green |
| 13 | G-11 | UC-2.3 | 按 postId 定位 | [036-ifv2-g-11-locate-message](../036-ifv2-g-11-locate-message/spec.md) | Draft；依赖 G-10a review-green |
| 14 | G-12 | UC-2.4 | 回复列表/分支 | [037-ifv2-g-12-reply-thread](../037-ifv2-g-12-reply-thread/spec.md) | Draft；依赖 G-11 review-green |
| 15 | G-13 | UC-3.1 | 会话已读 | [038-ifv2-g-13-channel-read](../038-ifv2-g-13-channel-read/spec.md) | Draft；依赖 G-12 review-green |
| 16 | G-14 | UC-3.2 | 单条已读 | [039-ifv2-g-14-post-read](../039-ifv2-g-14-post-read/spec.md) | Draft；依赖 G-13 review-green |
| 17 | G-14a | UC-3.3 | 模板已收到 | [040-ifv2-g-14a-template-received](../040-ifv2-g-14a-template-received/spec.md) | Draft；依赖 G-14 review-green |
| 18 | G-15a | UC-5.1 | 创建群聊 | [041-ifv2-g-15a-create-channel](../041-ifv2-g-15a-create-channel/spec.md) | Draft；依赖 G-14a review-green |
| 19 | G-15 | UC-5.2 | 创建话题 | [042-ifv2-g-15-create-topic](../042-ifv2-g-15-create-topic/spec.md) | Draft；依赖 G-15a review-green |
| 20 | G-16 | UC-5.3 | 关闭/退出群 | [043-ifv2-g-16-channel-close-leave](../043-ifv2-g-16-channel-close-leave/spec.md) | Draft；依赖 G-15 review-green |
| 21 | G-16b | UC-5.3b | member leave 广播 | [044-ifv2-g-16b-member-leave-broadcast](../044-ifv2-g-16b-member-leave-broadcast/spec.md) | Draft；依赖 G-16 review-green |
| 22 | G-16c | UC-5.4 | 群属性修改 | [045-ifv2-g-16c-channel-settings](../045-ifv2-g-16c-channel-settings/spec.md) | Draft；依赖 G-16b review-green |
| 23 | G-S1 | UC-4.1 | hello 全量增量 | [021-ifv2-g-s1-cold-start-sync](../021-ifv2-g-s1-cold-start-sync/spec.md) | Draft；依赖 G-16c review-green |
| 24 | G-S2 | UC-4.2 | sync notify | [022-ifv2-g-s2-offline-sync-notify](../022-ifv2-g-s2-offline-sync-notify/spec.md) | Draft；依赖 G-S1 review-green |
| 25 | G-S3 | UC-4.4 | 心跳 gap 补偿 | [023-ifv2-g-s3-pong-gap-recovery](../023-ifv2-g-s3-pong-gap-recovery/spec.md) | Draft；依赖 G-S2 review-green |
| 26 | G-S4 | UC-4.5 | 陌生 channel 兜底 | [024-ifv2-g-s4-unknown-channel-hydration](../024-ifv2-g-s4-unknown-channel-hydration/spec.md) | Draft；依赖 G-S3 review-green |

## Gap

| Gate | 处理 |
|---|---|
| G-03 | 文件/媒体上传；本轮 deferred-gap |
| G-17～G-27 | 19 个扩展 Gate；保留在产品 Gap Map，另开后续总控 |

## 完成公式

`Gate Green = Helix Green + Fixture/Hash Green + PC Mock/Real/Report Green + Mobile GTest/QuickJS Green + production-boundary Green + UC Review Green`

`Total Green = 26/26 Gate Green + v1FrameProducer=0 + v1FrameConsumer=0 + rawBusinessRenderFallback=0`

Total Green 之后只生成删除候选清单；用户选择删除范围后，必须使用新候选重新跑三仓 26 Gate。
