# InteractionFrame V2 全量切换与 V1 清退文档体系

> 本文范围排除 G03/G03a，不得将上传链路混入本次 Frame V2 清退。

本目录按 19 个已绑定业务 UC、25 个机器 Gate 描述切换前 V1、当前 V2、删除状态与验收入口，并额外收录尚未绑定机器 Gate 的会话列表行增量提案。当前生产合同只允许 `InteractionFrame/v2`；V1 producer、consumer、normalize、fixture 与正向兼容测试已经物理删除。

切换顺序是硬约束：**先让 Helix V2-only produce、cses-client V2-only consume，再跑首次纯 V2 E2E；首次验收通过后物理删除全部 V1，最后跑删除后二次 E2E。** 全部步骤均已完成：删除后 25 Gate 回归、余下 Task、规格/证据索引和 Spec Kit loop 已闭合。25/25 不是单独的完成条件，V1 物理删除与后续任务完成共同构成最终验收。

## 领域归属与依赖方向

- domain：Helix 决定消息排序、权限、未读、回执、身份收敛与完整 Surface VM，不依赖 HTTP、DB、Tauri 或 Angular。
- application：Helix 以 `Tick` 编排 `Effect`，并在 `PersistOk` 后调用 `SurfaceComposer`。
- infrastructure：Go HTTP/WS/PG 和 Helix SQLite driver 实现端口；Tauri 只传输 opaque Frame。
- interface：Angular codec/adapter 只做协议转换、整帧校验与渲染，不重建业务事实。
- 使用 State 表达恢复态，Adapter 隔离平台组件，Repository 隔离持久化，Strategy 仅用于真实的 Surface impact policy；不为每个 UC 复制新框架。

热路径按 mutation 单次遍历，slot 使用 `Map` 平均 `O(1)` 查找，因此一帧为 `O(m)` 时间、`O(m)` staging 空间；不得退化为 Surface 与 mutation 的无索引 `O(s*m)` 扫描。

## 导航

- [范围与版本](00-范围与源码版本.md)
- [全 UC 严格矩阵](01-全UC严格矩阵.md)
- [切换前 V1 公共架构（历史快照）](02-V1当前公共架构.md)
- [V2 目标公共架构](03-V2目标公共架构.md)
- [V1 下线清单](04-V1下线清单.md)
- [跨仓固定与验收](05-跨仓版本固定与验收.md)
- [协议与术语](06-协议与领域术语.md)
- [完整性检查](07-文档完整性检查.md)
- [当前代码差距与工具审计](08-当前代码差距与工具审计.md)
- [逐 Gate Spec 公共验收合同](09-逐Gate-Spec公共验收合同.md)

字段级设计先以 [G01 发文本](G01/17-G01字段级设计基线与规格输入.md) 为模板：类图落实 command、聚合、双库、WS、Frame、Surface、组件字段；时序图落实持久化屏障与回执；状态图落实竞态和跨 Surface 影响。后续 UC/GS 必须保持 CURRENT/TARGET 分栏，禁止把影响面清单直接写成已实现 mutation。

每个已绑定 UC 目录按 `11-V2切换Spec.md` → `12-V2切换Plan.md` → `13-V2切换Tasks.md` 顺序执行；`06`-`10` 保留组件、Frame、Go、数据库和下线细节。018 下的 `plan.md/tasks.md` 是 Spec Kit 聚合与 R2/R3 真源，各 Gate 的 `12/13` 是同一全局任务 ID 的就近执行视图。

## UC

- [GS01](GS01/README.md)：建立连接后完成全量增量恢复；Gate GS01
- [GS02](GS02/README.md)：按 cursor 补齐离线事件；Gate GS02
- [GS03](GS03/README.md)：发现心跳 gap 后执行有界恢复；Gate GS03
- [GS04](GS04/README.md)：补齐陌生频道并建立可操作 Surface；Gate GS04
- [G01](G01/README.md)：发送文本并收敛临时身份与服务端身份；Gate G01
- [G02](G02/README.md)：发送文档类型消息并按 typed VM 渲染；Gate G02
- [G04](G04/README.md)：在线撤回并在离线恢复中得到同一撤回态；Gate G04、G04a
- [G05](G05/README.md)：向多个目标频道转发消息；Gate G05
- [G06](G06/README.md)：提交快捷回复或 reaction 并展示权威聚合；Gate G06
- [G07](G07/README.md)：加急消息并收集确认事实；Gate G07
- [G08](G08/README.md)：创建定时消息；Gate G08
- [G09](G09/README.md)：取消本人定时消息；Gate G09
- [G10](G10/README.md)：加载首屏窗口并向前补窗；Gate G10、G10a
- [G11](G11/README.md)：定位指定消息并在提交后揭示；Gate G11
- [G12](G12/README.md)：打开回复线程并加载回复分支；Gate G12
- [G13](G13/README.md)：更新频道已读边界和未读摘要；Gate G13
- [G14](G14/README.md)：上报单条已读并确认模板已收到；Gate G14、G14a
- [G15](G15/README.md)：创建话题或新群聊并建立首批 Surface；Gate G15、G15a
- [G16](G16/README.md)：关闭、退出或修改频道治理属性；Gate G16、G16b、G16c
- [会话列表行增量提案](G17/README.md)：由全量 `replaceView` 演进为行增量；不是已用于频道置顶的 canonical `G-17`，不进入本次完成分母
