# Helix IM

`helix-im` 是运行在 Helix Core 上的确定性 MessageV3 IM 业务模块，负责消息领域状态机、
持久化屏障、恢复与业务事件生成，不包含平台 UI 或具体 I/O 实现。

```toml
[dependencies]
helix-im = "=0.1.18"
```

本项目采用 `MIT OR Apache-2.0` 双许可证。

## 合并转发投影

创建时 `forward_shape::merged_post` 保存来源摘要与 props；`query::render_ready::forward`
统一为历史查询与 live 事件输出摘要 `simpleMessage`、独立正文 `text` 和嵌套 `forwardDetail`。
递归投影复用现有来源项归一化，限制层数、每层条数及总节点数；异常嵌套局部返回 unavailable，
同层普通消息继续可用。具体上限以该模块常量为准，不改变持久化合并结构。

摘要回归同时覆盖历史查询、实时消息与会话 lastPost：图文正文 `11\n1\n1\n11`
和已保存摘要 `[图片] 3个 11 1 1 11` 独立透传，不由 Renderer 从正文重建媒体摘要。
当前源码已包含摘要和递归嵌套投影；客户端是否已使用该实现，须以客户端实际依赖与运行版本核验。
嵌套定向回归：`cargo test -p helix-im --lib query::render_ready::forward::tests`；客户端 Native 回归资产为 `e2e/bug-regression/nested-merged-forward.spec.ts`，验证外层 6 条记录中继续打开内层 3 条及完整图文。

新发送的多附件摘要将数量放在方括号内，以空格分隔，如 `[图片 3个] 你好`、`[视频 2个]`、`[媒体 2个] 你好`。单附件不显示数量，摘要总长仍为 50 字；历史保存的摘要不重写。

### simpleMessage 统一摘要（2026-09-10）

`src/message_summary.rs` 为摘要规则唯一入口：发送准备、公告发送、服务端入站落库、历史与实时末条、合并转发/逐条转发共享。多选转发为 `[聊天记录]`；媒体沿用 `[图片 3个] 文字` / `[视频 2个] 文字` / `[媒体 2个] 文字`；业务卡片标签和标题、语音、系统通知也在此生成。系统通知使用服务端姓名快照，不编码查看者专属的“您”。稀疏状态更新不制造摘要。接龙声明和条目投影携带 `simpleMessage`。客户端不得再重算。定向回归：`cargo test -p helix-im --lib message_summary`、`cargo test -p helix-im --test p1_send_build_test`。

频道末条补齐（2026-09-10）：`prepare_post` 同时处理对象/JSON 字符串形式的 `lastPost`。频道完整落库、PATCH、member 绝对态及 created/update 事件统一准备摘要；建群完成投影保留首条通知与末条时间。无末条的稀疏事件不造消息，不更改 @、加急、草稿字段。回归 `cargo test -p helix-im --lib channel_summary`。
