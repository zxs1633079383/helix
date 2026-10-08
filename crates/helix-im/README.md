# Helix IM

`helix-im` 是运行在 Helix Core 上的确定性 MessageV3 IM 业务模块，负责消息领域状态机、
持久化屏障、恢复与业务事件生成，不包含平台 UI 或具体 I/O 实现。

```toml
[dependencies]
helix-im = "=0.1.46"
```

本项目采用 `MIT OR Apache-2.0` 双许可证。

## 合并转发投影

`im_relay_messages` 经共享调用门面映射到 `im_create_posts`，提交
`post_ids,target_channel_ids,mode,command_id,req_id`；HTTP 请求只携带 ID 意图、模式和幂等键，
不读取本地源全文，也不接收客户端快照树。入口见 [`client_api/command.rs`](src/client_api/command.rs)
与 [`forward.rs`](src/forward.rs)。

历史、实时消息和会话 lastPost 共用 [`query/render_ready/forward.rs`](src/query/render_ready/forward.rs)
的有界 `props.forward` 卡片：`snapshotId,title,itemCount,preview`，preview 最多 3 项；
不递归输出 `forwardDetail`。详情通过 `im_forward_detail({root_post_id,path,offset,req_id})`
按页读取 `/posts/forwardDetail`，结果经 `im:read:result.data.body` 返回；
`im_cancel_forward_detail({req_id})` 取消模块等待并释放关联/计时器，不直接中断 HTTP socket。
边界与失败处理见 [`query/forward_detail.rs`](src/query/forward_detail.rs)。

定向回归入口：`cargo test -p helix-im --test forward_snapshot_test`、
`cargo test -p helix-im --lib query::render_ready::forward::tests`。
这里只说明现有实现与测试入口，未在文档日检中重跑；客户端消费情况须核对其实际依赖与运行版本。

新发送的多附件摘要将数量放在方括号内，以空格分隔，如 `[图片 3个] 你好`、`[视频 2个]`、`[媒体 2个] 你好`。单附件不显示数量，摘要总长仍为 50 字；历史保存的摘要不重写。

### simpleMessage 统一摘要（2026-09-10）

`src/message_summary.rs` 为摘要规则唯一入口：发送准备、公告发送、服务端入站落库、历史与实时末条、合并转发/逐条转发共享。多选转发为 `[聊天记录]`；媒体沿用 `[图片 3个] 文字` / `[视频 2个] 文字` / `[媒体 2个] 文字`；业务卡片标签和标题、语音、系统通知也在此生成。系统通知使用服务端姓名快照，不编码查看者专属的“您”。稀疏状态更新不制造摘要。接龙声明和条目投影携带 `simpleMessage`。客户端不得再重算。定向回归：`cargo test -p helix-im --lib message_summary`、`cargo test -p helix-im --test p1_send_build_test`。

频道末条补齐（2026-09-10）：`prepare_post` 同时处理对象/JSON 字符串形式的 `lastPost`。频道完整落库、PATCH、member 绝对态及 created/update 事件统一准备摘要；建群完成投影保留首条通知与末条时间。无末条的稀疏事件不造消息，不更改 @、加急、草稿字段。回归 `cargo test -p helix-im --lib channel_summary`。

### 本端消息完成关联

`im:post:sent.data.requestId` 可选，来自本端发送的 `req_id`，仅在权威 WS / sync 对账以及发送终态写入成功后释放。
`im:post:received.data.requestId` 可选，只在命中本端 pending forward 的请求及目标、原子提交和成员读回成功后释放；接收方、重复目标和 HTTP admission 不携带该成功关联。
Host 可按提交前登记的请求匹配最近转发目标；不得把 `im:post:batch-result` 的 accepted/pending 当作送达。字段不进入 Go Post、持久消息体或 core/driver 协议，旧消费者可忽略。

## 全局书签与本机历史

- `im_bookmark_load_global({page_number,page_size,req_id})` 独立于群内书签查询，
  调用 `post/bookmark/global`；认证用户/企业取运行时身份，分页为正整数且
  `page_size <= 100`、`page_number <= 1000000`。参数与 wire 真源见
  [`outbound/posts/read_ext.rs`](src/outbound/posts/read_ext.rs)，结果/失败回灌见
  [`port_reply/dispatch_read.rs`](src/port_reply/dispatch_read.rs)。
- `im_bookmark_count({req_id})` 返回当前认证用户/企业的可见书签总数，使用会话内缓存，
  不写 SQLite。首次读取、断连失效后的读取或五分钟过期后的读取通过 `post/bookmark/count`
  校准；并发等待合并为一次 HTTP，结果复用 `im:read:result`。
  书签创建/删除按服务端 `affectedCount` / `countDelta` 更新已知总数；缺字段、身份不符或
  不合法增量使缓存失效，不凭 `SUCCESS` 猜计数。与写入重叠的旧校准响应不覆盖新状态；
  校准失败返回错误并退避 30 秒，身份切换/stop 取消等待者。实现见
  [`bookmark_count.rs`](src/bookmark_count.rs)，回归入口为
  `cargo test -p helix-im --test read_relay_test bookmark_count`。
- `im_recent_history` 的 `kind` 支持 `search`、`forward`、`emoji-send`、`emoji-reaction`，
  `action` 支持 `read`、`record`、`remove`、`clear`、`import`；表情发送与回应历史分开保存。
  写请求回传 read 返回的 `scope`，身份由运行时生成；通过既有 `im_recent_history` 表持久化，
  写入成功后返回新状态。完整 payload 与校验见 [`recent_history.rs`](src/recent_history.rs)。

以上为源码合同核对，不代表远端接口或客户端 UI 已在本轮验收。

## 群成员稀疏更新与显式清空

[`channel/write/members.rs`](src/channel/write/members.rs) 按字段合并成员来源：当前 `members`
优先于角色摘要，角色摘要优先于历史 `memberChange.join`。缺省字段保留已有值；
`nickName: ""` 明确清空昵称，缺少或 null 昵称不生成清空。`boss[].option=leave` 只撤销
BOSS 角色，成员离场由 `memberChange.leave` 表达，不能从空角色摘要推断离场。

[`acl/to_effect_s1/storage.rs`](src/acl/to_effect_s1/storage.rs) 根据 `omitted_fields` 将成员
分为最多 8 个固定字段形状批次，通过 `exclude_from_update` 保留冲突行中的缺省字段；
默认值只用于新行。定向回归入口：
`cargo test -p helix-im --test channel_empty_semantics_regression_test`、
`cargo test -p helix-driver-host --test channel_empty_semantics_sqlite`。
上述测试入口已核对源码与 Cargo 注册，本次纯文档日检未执行 Rust 或真实业务链验收。
