# Phase1-4 客户端与 Helix Gate 时序图

本目录以 Spec 047 的 phase -> domain -> gate 为目录真源，按发送消息参考图的粒度描述
cses-client（Angular -> Tauri -> Helix）的实际入口、Helix Tick/Effect/PortReply、Go
HTTP/WS、租户 SQLite，以及 Native/FFI/Web 的原样事件交付边界。

## 证据口径

- PC 参考客户端：/System/Volumes/Data/workspace/angular/cses-client。
- Helix Gate 分母来自
  specs/047-im-event-gate-inventory/contracts/gate-migration.v1.json，P1-P4 共 22 个
  Gate；不要与 Go 侧 Phase1-4 的 84 Gate 混用。
- Effect::Emit {event,data} 是唯一业务出站；Native im:__bus__、FFI batch 和 Web
  EventBatch 只是承载同一 raw bytes。
- 图中的 HTTP 2xx、invokeAccepted、Rust deterministic replay、静态 command 注册均不是
  真实三端完成证据。当前 PC/Mobile/Web consumer sync 与本地 .v1 generated artifacts
  仍分别是 NOT_SYNCED_BY_SCOPE / NOT_VERIFIED。
- Spec/旧测试中出现的 im:read:result、旧 Frame/Surface 或 post/posted 命名，若与当前
  producer 冲突，图中以当前 Helix 代码为准，并在图内注明 drift。

## 目录

| Phase | Domain | Gate |
|---|---|---|
| [Phase1](./Phase1/) | [channel](./Phase1/channel/) | [G-15a 创建群聊](./Phase1/channel/Gate-15a-创建群聊.mmd) |
| [Phase1](./Phase1/) | [post](./Phase1/post/) | [G-01 发送文本消息](./Phase1/post/Gate-01-发送文本消息.mmd) |
| [Phase2](./Phase2/) | [post](./Phase2/post/) | [G-02 文档](./Phase2/post/Gate-02-发送文档消息.mmd)、[G-03a 重试](./Phase2/post/Gate-03a-失败消息重试.mmd)、[G-04 在线撤回](./Phase2/post/Gate-04-在线撤回消息.mmd)、[G-04a 离线撤回](./Phase2/post/Gate-04a-离线同步撤回.mmd)、[G-05 转发](./Phase2/post/Gate-05-转发与批量结果.mmd)、[G-06 reaction](./Phase2/post/Gate-06-reaction绝对态.mmd)、[G-07 加急](./Phase2/post/Gate-07-加急与确认.mmd)、[G-14a 模板已收到](./Phase2/post/Gate-14a-模板消息已收到.mmd) |
| [Phase2](./Phase2/) | [read](./Phase2/read/) | [G-14 单条已读](./Phase2/read/Gate-14-单条消息已读回执.mmd) |
| [Phase3](./Phase3/) | [schedule](./Phase3/schedule/) | [G-08 创建定时](./Phase3/schedule/Gate-08-创建定时消息.mmd)、[G-09 取消定时](./Phase3/schedule/Gate-09-取消定时消息.mmd) |
| [Phase4](./Phase4/) | [timeline](./Phase4/timeline/) | [G-10 latest](./Phase4/timeline/Gate-10-最新时间线.mmd)、[G-10a older](./Phase4/timeline/Gate-10a-加载older历史.mmd)、[G-10b newer](./Phase4/timeline/Gate-10b-加载newer消息.mmd)、[G-10c 满窗追加](./Phase4/timeline/Gate-10c-满窗口追加与淘汰.mmd)、[G-10d 离底提示](./Phase4/timeline/Gate-10d-离底anchor与新消息提示.mmd)、[G-11 已缓存定位](./Phase4/timeline/Gate-11-已缓存消息定位.mmd)、[G-11a 未缓存定位](./Phase4/timeline/Gate-11a-未缓存定位与双向分页.mmd)、[G-12 回复线程](./Phase4/timeline/Gate-12-回复列表与话题分支.mmd) |
| [Phase4](./Phase4/) | [read](./Phase4/read/) | [G-13 会话已读](./Phase4/read/Gate-13-当前viewer会话已读.mmd) |

每张图都保留 UI -> command/inbound -> Tick -> helix-im -> Effect -> Host/SQLite/Go ->
PersistOk -> Emit -> clients 的逐跳链路；需要判断真实运行态时，应继续回到
cses-client、Go 服务和对应 Gate 的 runtime evidence，而不是只看图。

发送消息的原始参考图仍保留在 [发送消息.mmd](./发送消息.mmd)；Phase1/post/G-01 是按当前
producer 与测试事实补全后的 Gate 版本。
