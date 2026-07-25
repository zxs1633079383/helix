# InteractionFrame V2 26 Gate 三仓纵向闭环总 Spec 索引

> 总控 Feature：[020-ifv2-three-repo-uc-control](../../specs/020-ifv2-three-repo-uc-control/spec.md)
>
> 机器索引：[gate-spec-manifest.json](../../specs/020-ifv2-three-repo-uc-control/gate-spec-manifest.json)

## 范围

- Active 26 Gate：G01 基线 + G02 试点 + G03a + G04-G16c + GS1-GS4。
- G03 文件/媒体上传进入 Gap。
- G17-G27 的 19 个扩展 Gate 进入 Gap。
- 一个 UC 完成三仓纵向闭环和 UC Review 后，才能进入下一个。
- 全部 26 Gate Green 后只生成 V1/legacy 删除候选清单，删除范围由用户决定。

## 三仓硬合同

1. Helix 是业务逻辑和 canonical Fixture 唯一真源。
2. PC 必须完成 Fixture parity、Mock、Real、Surface/DOM 断言与报告。
3. Mobile 必须完成 mirror parity、链接生产 SDK/Bridge/SurfaceRuntime/QuickJS 的 GTest 与 typed view。
4. Mock/GTest 不得补生产代码缺失的业务字段、排序、权限或 reducer。
5. 每个 Gate 必须通过行为、代码边界、Frame V2 与证据一致性 Review 才能 Green。
6. 删除前与删除后是两次不同候选、不同证据，不得复用旧 Green。

## Gate -> Spec Kit

| 顺序 | Gate | 业务 | 当前代码文档 | Spec Kit Feature |
|---:|---|---|---|---|
| 1 | G-01 | 发送文本 | [G01](./G01/README.md) | [019 G01 基线](../../specs/019-g01-wdio-five-phases/spec.md) |
| 2 | G-02 | 发文档 | [G02](./G02/README.md) | [025-ifv2-g-02-document-message](../../specs/025-ifv2-g-02-document-message/spec.md) |
| 3 | G-03a | 重发失败 | Strict Matrix；plan 阶段补当前代码 discovery | [026-ifv2-g-03a-retry-failed-send](../../specs/026-ifv2-g-03a-retry-failed-send/spec.md) |
| 4 | G-04 | 撤回 | [G04](./G04/README.md) | [027-ifv2-g-04-online-revoke](../../specs/027-ifv2-g-04-online-revoke/spec.md) |
| 5 | G-04a | 离线撤回重放 | [G04](./G04/README.md) | [028-ifv2-g-04a-offline-revoke](../../specs/028-ifv2-g-04a-offline-revoke/spec.md) |
| 6 | G-05 | 转发/合并转发 | [G05](./G05/README.md) | [029-ifv2-g-05-forward-message](../../specs/029-ifv2-g-05-forward-message/spec.md) |
| 7 | G-06 | 快捷回复 emoji | [G06](./G06/README.md) | [030-ifv2-g-06-quick-reaction](../../specs/030-ifv2-g-06-quick-reaction/spec.md) |
| 8 | G-07 | 加急 + 确认 | [G07](./G07/README.md) | [031-ifv2-g-07-urgent-message](../../specs/031-ifv2-g-07-urgent-message/spec.md) |
| 9 | G-08 | 定时消息 | [G08](./G08/README.md) | [032-ifv2-g-08-schedule-create](../../specs/032-ifv2-g-08-schedule-create/spec.md) |
| 10 | G-09 | 取消定时 | [G09](./G09/README.md) | [033-ifv2-g-09-schedule-cancel](../../specs/033-ifv2-g-09-schedule-cancel/spec.md) |
| 11 | G-10 | 切群首屏 | [G10](./G10/README.md) | [034-ifv2-g-10-timeline-window](../../specs/034-ifv2-g-10-timeline-window/spec.md) |
| 12 | G-10a | 上拉更早历史 | [G10](./G10/README.md) | [035-ifv2-g-10a-load-older](../../specs/035-ifv2-g-10a-load-older/spec.md) |
| 13 | G-11 | 按 postId 定位 | [G11](./G11/README.md) | [036-ifv2-g-11-locate-message](../../specs/036-ifv2-g-11-locate-message/spec.md) |
| 14 | G-12 | 回复列表/分支 | [G12](./G12/README.md) | [037-ifv2-g-12-reply-thread](../../specs/037-ifv2-g-12-reply-thread/spec.md) |
| 15 | G-13 | 会话已读 | [G13](./G13/README.md) | [038-ifv2-g-13-channel-read](../../specs/038-ifv2-g-13-channel-read/spec.md) |
| 16 | G-14 | 单条已读 | [G14](./G14/README.md) | [039-ifv2-g-14-post-read](../../specs/039-ifv2-g-14-post-read/spec.md) |
| 17 | G-14a | 模板已收到 | [G14](./G14/README.md) | [040-ifv2-g-14a-template-received](../../specs/040-ifv2-g-14a-template-received/spec.md) |
| 18 | G-15a | 创建群聊 | [G15](./G15/README.md) | [041-ifv2-g-15a-create-channel](../../specs/041-ifv2-g-15a-create-channel/spec.md) |
| 19 | G-15 | 创建话题 | [G15](./G15/README.md) | [042-ifv2-g-15-create-topic](../../specs/042-ifv2-g-15-create-topic/spec.md) |
| 20 | G-16 | 关闭/退出群 | [G16](./G16/README.md) | [043-ifv2-g-16-channel-close-leave](../../specs/043-ifv2-g-16-channel-close-leave/spec.md) |
| 21 | G-16b | member leave 广播 | [G16](./G16/README.md) | [044-ifv2-g-16b-member-leave-broadcast](../../specs/044-ifv2-g-16b-member-leave-broadcast/spec.md) |
| 22 | G-16c | 群属性修改 | [G16](./G16/README.md) | [045-ifv2-g-16c-channel-settings](../../specs/045-ifv2-g-16c-channel-settings/spec.md) |
| 23 | G-S1 | hello 全量增量 | [GS01](./GS01/README.md) | [021-ifv2-g-s1-cold-start-sync](../../specs/021-ifv2-g-s1-cold-start-sync/spec.md) |
| 24 | G-S2 | sync notify | [GS02](./GS02/README.md) | [022-ifv2-g-s2-offline-sync-notify](../../specs/022-ifv2-g-s2-offline-sync-notify/spec.md) |
| 25 | G-S3 | 心跳 gap 补偿 | [GS03](./GS03/README.md) | [023-ifv2-g-s3-pong-gap-recovery](../../specs/023-ifv2-g-s3-pong-gap-recovery/spec.md) |
| 26 | G-S4 | 陌生 channel 兜底 | [GS04](./GS04/README.md) | [024-ifv2-g-s4-unknown-channel-hydration](../../specs/024-ifv2-g-s4-unknown-channel-hydration/spec.md) |

## Gap

| 范围 | 数量 | 本轮处理 |
|---|---:|---|
| G-03 | 1 | 文件/媒体上传 deferred-gap |
| G-17～G-27 | 19 | 产品扩展 Gap；只登记，不进入 Active 分母 |
