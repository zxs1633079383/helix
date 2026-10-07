# 统一错误来源交接
更新时间：2026-10-07 Asia/Shanghai

## 目标与边界
- 用户要求失败能追溯 HTTP 路径/WS action、来源 Command 和请求，并保留原始业务原因。
- 本仓实现与本地验证已授权；未提版、未发布、未 push，PRE/桌面未安装候选版本。
- 共享 helix-im 收口；core/driver 无业务判断。成功事件不变；机器错误码保留。

## 代码身份
- 仓库：/System/Volumes/Data/workspace/angular/visual/helix
- 分支：reactor/MessageV3-helix；基线 000d83a6312e90977526c36b43672e297e7a6b02。
- 本批次身份以包含此文件的提交及其父提交 diff 为准；无关未跟踪文档/产物未纳入。

## 实现与生命周期
- `src/failure_context.rs` 在 Module Command/PortReply/Timer/WS 边界保存元数据，关联后续 HTTP/Persist。
- HTTP200 的显式 failed/failure/error/fail 在读取 data 前拒绝；非2xx保留状态及可解出的业务原因。
- 普通 error 字符串增加 command/source/stage/req_id/corr；机器码增加 errorContext。
- 固定字段的 `*-failed` 投影保持合同，来源进入统一结构化日志；建群/分类专用拒绝码保持。
- HTTP 原因与原有领域错误同时保留；CoreError source 链保留可 downcast 的 ImError。
- 活跃关联在回包/Timer触发或取消时释放；stop/身份切换清空。晚到 WS 可查最近256条诊断历史。
- 历史仅保存有界来源，无 body/headers，不恢复业务请求；淘汰后不能证明来源则标 server_push。
- URL 去 authority/query/fragment；错误文本限长并过滤常见凭据模式，不输出完整响应。
- 入口/终态沿用既有 Command；无新增业务 Command。读错误不落库，写/WS仍等既有持久屏障。
- 回放覆盖 HTTP业务/传输失败、Persist失败、乱序、重复/迟到、stop、timer取消、WS关联和历史容量。

## 故障证据
- PRE 16:54:43：HTTP200，业务 failed / ent: channel_member not found。
- trace 18dc336a48c20d0d；query trace 18dc336a48be2273；req_id mv3-hydration-1791363283365-ahn2gm9a。
- 请求 /api/cses/channel/load/incrementByChannelId；此修复改善错误诊断，不改变成员权限或修复成员数据。

## 本地验证
- 公开 seam 红测4项先失败：/tmp/helix-failure-context-red.log。
- 最终聚焦546项通过（含462 lib、10新集成、分类/转发/WS及受影响Gate）：/tmp/helix-failure-context-final-focused.log。
- 五crate check、Web wasm build通过：/tmp/helix-failure-context-final-{check,wasm}.log。
- core测试通过、runtime依赖匹配为空：/tmp/helix-failure-context-{core,core-tree}.log。
- 热路径审计：/tmp/helix-failure-context-audit-all.log；新告警为Source元数据Arc/静态Cow克隆，非payload深拷贝。
- 其余脚本告警位于未改动的既有函数；无新增SQL/存储结构或逐条跨平台回调。
- 全量与Gate生成结果：见下一节；不会宣称仓库全绿或真实运行态通过。

## 其余验证与未完成边界
- 最终全量日志：/tmp/helix-failure-context-final-all-im.log。
- 剩余失败：旧 phase1 MV3-G03a artifact 的 go字段不合schema；MV3-G01a/G01f期待HTTP readback；MV3-G07b公告摘要前缀。
- 上述断言/对应业务逻辑未在本批次修改；未运行基线隔离回放，不把归因判断写成基线实测。
- 30 Gate producer已运行并生成；整体生成被旧phase1 schema失败阻断：/tmp/helix-failure-context-gates.log。
- cutover --require-generated --self-test通过；consumerSync=NOT_SYNCED_BY_SCOPE：/tmp/helix-failure-context-cutover.log。
- 真实Go/Native/UI候选验收未运行；本轮用现场响应回放，不等同真实链。
- 下一步：发布需单独授权；成员缺失本身需另行核查成员生命周期，不能靠此错误处理恢复访问权限。
