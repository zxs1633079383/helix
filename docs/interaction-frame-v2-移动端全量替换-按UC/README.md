# Mobile InteractionFrame V2 全量替换：历史 Spec 输入

> `RETIRED_HISTORICAL`。审计时间：2026-07-24 PDT。本文档集记录创建正式 Mobile Spec 前的
> 源码事实与目标边界，不是当前 Spec、Plan、Tasks，也不把 PC 018 的静态硬切升级为移动端或
> 删除后二次 E2E 完成。当前 MessageV3 实施与 Gate 真源见
> [`../../specs/047-im-event-gate-inventory/`](../../specs/047-im-event-gate-inventory/)。

## 结论先行

范围是 **19 个业务 UC、25 个机器 Gate**，明确排除 `G03/G03a`。最新代码必须分三层判断：

- Helix 018 worktree HEAD `57d2ddab` 已将唯一 builder/emitter 收敛到
  `InteractionFrameV2`，默认 schema 为 2，并物理删除 V1 类型、常量和 fixture。
- cses-client 018 worktree HEAD `846afd3fa` 已钉住上述 Helix revision；Native ingress 与
  Angular codec 均只接受完整 V2。它是 PC consumer 对照，不等于 Mobile 已切换。
- Mobile 工作树正在扩展 schema 2 的 rich VM 校验和 attach/resync 回送，但 C++ frame 根结构仍是
  `{schemaVersion,transactionId,mutations}`，没有正式 V2 的 `runtime/directives`，mutation 也没有
  `surfaceEpoch`；`message.composer` 仍不在封闭 `SurfaceId` 中。
- Mobile 同时接受 V1/V2，Venus 仍有 raw DomainEvent 广播面，实际产品 QuickJS 页面源码仍
  `SOURCE_MISSING`。
- Mobile 的 source lock、command contract 与 UC manifest 当前仍固定 Helix `477ca58`，没有
  固定到 018 删除提交 `57d2ddab`。
- 018 的删除后二次 25-Gate Native 回归 `T092-T096` 仍未完成，R4 仍是 blocked；不得把
  静态 V2-only 与 focused PASS 写成产品全链 Green。

因此正式 Mobile Spec 应以 018 的 Helix V2-only producer 为上游基线，只实施 Mobile pin、
strict V2 consumer、QuickJS/raw fallback 清退与移动端 25-Gate 验收，不再重复规划 Helix V1
producer 迁移。

## 阅读顺序

1. [范围、版本与 pin](./00-范围与跨仓源码版本.md)
2. [当前代码差距](./08-当前代码差距与CONTRACT_DRIFT.md)
3. [19 UC / 25 Gate 矩阵](./01-移动端25-Gate严格矩阵.md)
4. [当前 Mobile 架构](./02-Venus-Mobile当前公共架构.md)
5. [目标 Mobile 架构](./03-Mobile-Frame-V2目标公共架构.md)
6. [V1 下线边界](./04-V1公共下线清单.md)
7. [候选固定与验收](./05-跨仓版本固定与验收.md)
8. [协议术语](./06-Mobile-QuickJS-C++协议与术语.md)
9. [公共验收合同](./09-全Gate公共验收合同.md)

共享的 Helix、Go、PG、SQLite 与逐 Gate 领域合同只链接
[PC/Helix 文档真源](../interaction-frame-v2-全量替换-按UC/README.md)，本目录不再复制。

## 业务 UC

| 业务 UC | 机器 Gate | Mobile 平台增量合同 |
|---|---|---|
| [GS01](./GS01/README.md) | GS01 | 连接后全量恢复 |
| [GS02](./GS02/README.md) | GS02 | notify/cursor 离线恢复 |
| [GS03](./GS03/README.md) | GS03 | pong gap 补偿 |
| [GS04](./GS04/README.md) | GS04 | 陌生频道 hydrate |
| [G01](./G01/README.md) | G01 | 文本发送与身份收敛 |
| [G02](./G02/README.md) | G02 | 文档消息 |
| [G04](./G04/README.md) | G04、G04a | actor/observer 撤回 |
| [G05](./G05/README.md) | G05 | 多目标转发 |
| [G06](./G06/README.md) | G06 | reaction/快捷回复 |
| [G07](./G07/README.md) | G07 | 加急与确认 |
| [G08](./G08/README.md) | G08 | 创建定时消息 |
| [G09](./G09/README.md) | G09 | 取消定时消息 |
| [G10](./G10/README.md) | G10、G10a | 首屏/向前补窗 |
| [G11](./G11/README.md) | G11 | 定位消息 |
| [G12](./G12/README.md) | G12 | 回复线程 |
| [G13](./G13/README.md) | G13 | 频道已读 |
| [G14](./G14/README.md) | G14、G14a | 单条回执/模板确认 |
| [G15](./G15/README.md) | G15、G15a | 话题/群聊创建 |
| [G16](./G16/README.md) | G16、G16b、G16c | 关闭/退出/设置 |

## 规格边界

- domain/application：Helix 拥有事实、排序、权限、ActionBinding、Effect、Persist barrier 和完整
  Surface VM；不依赖 QuickJS/C++/FFI。
- infrastructure/interface：C++/Venus 只传输 envelope/frame，整帧校验并原子交换 typed VM；
  QuickJS 只提交 intent、读取 VM 和渲染。
- 一帧 apply 对 mutation 单次遍历，目标复杂度为 `O(m)` 时间与 `O(m)` staging 空间；slot
  以 `(surface, canonical_scope)` 索引，禁止 UI 侧做无索引 `O(surface * mutation)` 对账。
- 正式 Spec/Plan/Tasks 只在 `specs/` 产生。本目录每个业务 UC 仅保留一个平台增量入口，避免
  重新形成 25 Gate × Spec/Plan/Tasks 的平行真源。

Mobile 切换顺序唯一为：

`固定 018 或其后继 Helix candidate -> Mobile 禁收 V1/禁 raw fallback -> 首轮移动端纯 V2 -> 物理删除 Mobile legacy reader/fixture/reducer -> 新 candidate 二次纯 V2`

`schemaVersion` 字段保留且 V2 必须为 `2`；不得误删独立版本域
`InteractionProtocol/v1` 或 `ActionInvocationEnvelope/v1`。
