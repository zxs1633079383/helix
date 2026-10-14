# 会话未读边界透传

当前分支 `reactor/MessageV3-helix`，基线 `23a4f80a`，当前工作区实施；不提交、不提版、不发布。

## 合同和生命周期

- `im_channels_view` 远程写命令继续复用 `/channels/view`，payload 为 `channels:[{id,isRoot:true,throughPostId?}]`。省略边界保留传统全清；`channelIds` 便捷输入仍归一为无边界对象。
- `throughPostId` 显式存在时必须通过 canonical post ID 校验（Java 24位、Go 26位）；null、空、临时 ID、错误类型拒绝，不能降级全清。消息归属、可见性与边界计数由 Go 权威校验。
- 两层校验：public `build_command` normalizer 与公开 `Tick::Command` outbound builder；未知字段、非root、重复频道仍拒绝。
- HTTP 仅受理；会话绝对态沿既有 `update_channel` / 成员投影及 matching PersistOk 发布。不在 Helix 推算未读数、不修改单条成员回执。
- HTTP/业务失败、超时、取消沿既有 read relay；本次不新增 pending、timer、correlation 或 I/O。PersistErr 不发布成功，重复 PersistReply 不重复发布；无新资源需清理。
- 无新增命令、存储 schema、平台分叉、缓存或逐消息扫描。三端共享业务实现，不改 core/driver。

## 验证

- 公开 Tick 新有效边界用例先红：旧 builder 拒绝额外字段；`artifacts/unread-boundary-20261014/red.log`。
- 公开 normalizer 定向 5 项通过（原 channelIds、原wire、拒绝重复/非法channel、边界有效与非法）。
- G13 定向涵盖边界HTTP透传、HTTP不产生绝对态、matching持久屏障、重复回执、非法边界无HTTP、带边界PersistErr；另保留原全清成功与回应查询/ACK回归。
- 五crate检查、core测试、WASM与IM全量结果见下方收尾追加。
- 真实 Go/native/FFI/Web 业务与UI验收 NOT_VERIFIED，由客户端主任务衔接。未宣称5000人大群负载已经验证。

### 最终结果

- G13 16/16、公开入口5/5通过；五crate check通过，core 28项+1 doc通过（1既有doc ignored），WASM build通过。core依赖树未匹配tokio/sqlx/reqwest/web-sys/tauri。
- IM全量在lib阶段481通过、1失败后停止，失败为未修改的 `send/upload_props/plan.rs:513 invalid_server_metadata_boundaries_are_rejected_before_planning`：67,108,865字节文件被允许，但旧测试预期拒绝；未扩修、未声称全量绿色。
- 相关日志位于 `artifacts/unread-boundary-20261014/{public,g13,check,core,wasm,full}.log`。
