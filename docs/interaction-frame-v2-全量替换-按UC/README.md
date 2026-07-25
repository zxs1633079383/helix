# InteractionFrame V2 当前代码对接：按 UC

> 源码审计时间：2026-07-24 PDT。本目录只描述当前生产代码，不再保存 Frame V1 历史页，
> 也不复制已完成的 Spec/Plan/Tasks。正式实施历史与验收任务见
> [018 InteractionFrame V2 cutover](../../specs/018-interaction-frame-v2-cutover/)。

## 结论

- Helix Frame producer 是 V2-only：唯一公开类型为 `InteractionFrameV2`，常量
  `INTERACTION_FRAME_SCHEMA_VERSION=2`，一个 `Effect::Emit` 携带一个完整 Frame。
- cses-client Native sink 只接受 `schemaVersion=2` 并 opaque 转发；Angular
  `decodeInteractionFrame` 同样 fail-closed，随后由 `SurfaceRuntime` 原子 apply。
- Frame V1 type/codec/writer/reader/normalize/fixture 已从当前生产树物理消失，因此各 UC 的
  V1 文档已下线；当前 V2 类图、流程图与分层合同仍按 UC 保留。
- 交互入口还不是全量 ActionBinding。当前既有 typed `ActionBinding -> im_invoke_action`，
  也有 `ShellService -> HELIX_COMMAND_NAMES` direct command；每个 UC 页按实际代码标明。
- `ActionInvocationEnvelope/v1`、`strict-matrix.v1.json` 和测试报告 `schemaVersion=1`
  是独立版本域，不是 Frame V1，禁止误删。

## 依赖方向

`MessageV4 intent -> CommandPort/ActionInvocation -> Tauri transport -> Helix Module -> Effect -> PersistOk/authority read -> InteractionFrameV2 -> Native opaque sink -> ProjectionBus -> ProjectionStore -> SurfaceRuntime -> typed component`

Helix 拥有业务事实、权限、排序、ActionBinding 与完整 Surface VM；Angular 只保留 route、draft、
overlay 和诊断等 shell state，不从 raw event 重建业务事实。

## 阅读顺序

1. [源码核验与版本](./00-源码核验与版本.md)
2. [19 UC / 25 Gate 当前矩阵](./01-按UC当前代码矩阵.md)
3. [V2 当前公共架构](./02-V2当前公共架构.md)
4. [协议版本边界](./03-协议版本边界.md)
5. [验收证据边界](./04-验收证据边界.md)
6. [文档完整性检查](./05-文档完整性检查.md)
7. [G02-G15 FrameV2 字段与验收总索引](./06-G02-G15-FrameV2字段与验收总索引.md)
8. [26 Gate 三仓纵向闭环总 Spec 索引](./07-26Gate三仓纵向闭环总Spec索引.md)

## 业务 UC

| UC 文件夹 | 机器 Gate | 当前业务 |
|---|---|---|
| [GS01](./GS01/README.md) | G-S1 | 连接建立后的全量增量恢复 |
| [GS02](./GS02/README.md) | G-S2 | sync notify 离线恢复 |
| [GS03](./GS03/README.md) | G-S3 | pong gap 有界补偿 |
| [GS04](./GS04/README.md) | G-S4 | 陌生频道 hydrate |
| [G01](./G01/README.md) | G-01 | 发送文本 |
| [G02](./G02/README.md) | G-02 | 发送文档消息 |
| [G04](./G04/README.md) | G-04、G-04a | 在线撤回与离线撤回重放 |
| [G05](./G05/README.md) | G-05 | 单条与多选转发 |
| [G06](./G06/README.md) | G-06 | reaction / 快捷回复 |
| [G07](./G07/README.md) | G-07 | 加急与确认 |
| [G08](./G08/README.md) | G-08 | 创建定时消息 |
| [G09](./G09/README.md) | G-09 | 取消定时消息 |
| [G10](./G10/README.md) | G-10、G-10a | 首屏窗口与向前补窗 |
| [G11](./G11/README.md) | G-11 | 按 postId 定位 |
| [G12](./G12/README.md) | G-12 | 回复线程与分支 |
| [G13](./G13/README.md) | G-13 | 频道已读 |
| [G14](./G14/README.md) | G-14、G-14a | 单条已读与模板已收到 |
| [G15](./G15/README.md) | G-15、G-15a | 创建话题与群聊 |
| [G16](./G16/README.md) | G-16、G-16b、G-16c | 频道关闭、退出与设置 |

当前代码文档矩阵仍是 25 Gate 历史切换口径；新的三仓实施总控将 `G03a` 失败重发纳入
Active，因此总分母为 26，入口见上方总 Spec 索引。`G03` 文件/媒体上传继续进入 Gap。
`G17-G27` 的 19 个扩展 Gate 同样进入 Gap，不与本轮逐 UC 实施混算。
