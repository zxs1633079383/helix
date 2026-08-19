# MessageV3 跨层源码地图

这份地图只提供检索入口，不取代当前源码。Helix、cses-client 和 Go 仓库可能漂移；开始排查时用符号名和事件常量重新确认调用点，并记录当前 `file:line`。

## 一、当前链路的典型 owner

| 跳点 | 当前常见位置 | 要确认的事实 |
|---|---|---|
| Angular Command 调用 | `cses-client/src/pages/message-v3/service/message.service.ts`、`service/dialog-switch/dialogSwitch.service.ts` | UI 是否通过 MessageV3 Port 调 Command，而非旧 IPC 旁路 |
| Angular Command adapter | `cses-client/src/pages/message-v3/service/im-gate/helix/helixImCommand.adapter.ts` | payload、字段命名、`invokeAccepted`/request id |
| Tauri invoke | `cses-client/src-tauri/src/features/helix_im/runtime.rs`、对应 command 注册处 | IPC 是否进当前 Helix runtime，是否被旧实现拦截 |
| Helix Command 路由 | `crates/helix-im/src/module/trait_impl.rs`、`src/client_api/command.rs` | `Tick::Command` 名称、公开入口、首批 Effect 和 correlation |
| HTTP/WS outbound | `crates/helix-im/src/outbound/` | 请求 body/headers、HTTP/WS correlation、response parser |
| WS action handler | `crates/helix-im/src/ws/handlers/` | action 白名单、raw `data` 是否原样进入业务 handler |
| Persist 编排 | `crates/helix-im/src/ws/handlers/channel_member_update.rs`、`src/port_reply/dispatch_*.rs` | `queue_*_persist`、`Effect::PersistAtomic`、成功/失败终态 |
| Projection builder | `crates/helix-im/src/port_reply/dispatch_channel_create.rs` 等 | 是否从 authority/readback 复制所有事件合同字段 |
| Event constructor | `crates/helix-im/src/event/channel.rs`、`src/event/mod.rs` | `{event,data}` envelope、`MessageV3Event::into_effect()` |
| Host dispatch | `crates/helix-driver-host/src/dispatch.rs`、`src/engine.rs` | `OwnedEffect::Emit` 是否交给 EventSink，是否 flush/批处理 |
| Native sink | `crates/helix-driver-native/src/event_sink.rs` | `NativeEventSink` receiver、BUS channel、send/lag/no-receiver 行为 |
| Tauri bridge | `cses-client/src-tauri/src/features/helix_im/runtime.rs`、`event_sink.rs` | broadcast 到 `app.emit_str("im:__bus__", ...)` 的序列化和 guard |
| Angular event adapter | `cses-client/src/pages/message-v3/service/im-gate/helix/helixImEvent.adapter.ts` | envelope 解包、event→kind 映射、payload source 选择、类型收窄 |
| Angular IPC/service | `cses-client/src/pages/message-v3/service/ipcEvent.service.ts` | 是否丢 undefined、是否按 channel id upsert/去重 |
| Angular handler/render | `cses-client/src/pages/message-v3/service/ipcEventHandler.service.ts`、`src/pages/message-v3/utils/` | source 解析、scope 过滤、view model/DOM 结果 |

## 二、建群 `channel_created` 示例

典型静态路径：

```text
im_create_channel
  → module/trait_impl.rs
  → outbound/channel_admin/existing.rs
  → Go channel_created WS frame
  → ws/handlers/channel_created.rs
  → ws/handlers/channel_member_update.rs::queue_channel_create_persist
  → Effect::PersistAtomic
  → port_reply/dispatch_channel_create.rs::handle_channel_create_persist_reply
  → build_message_v3_created_projection
  → event::channel::created
  → MessageV3Event::into_effect
  → host dispatch → NativeEventSink
  → client runtime/event_sink.rs → im:__bus__
  → helixImEvent.adapter.ts → ipcEvent.service.ts
  → ipcEventHandler.service.ts
```

排查 `source.type` 时必须同时确认：

1. Go raw frame 的 `data.source` 是对象而不是字符串或空值；
2. WS handler 传给 persist 的 channel JSON 仍然有 `source`、`createAt`、`createBy`；
3. `build_message_v3_created_projection` 把这些字段放入 `{event,data}`，不能只依赖 durable query 之后再补；
4. EventSink receiver 和 Tauri bus 的同一 envelope 仍保留对象形状；
5. Angular `parseDialogSource`/adapter 不把对象误当成旧字符串 source；
6. SQLite read-back 的字段与 created event projection 合同一致，或者明确记录两者的有意差异。

## 三、搜索模板

在当前仓库先用符号/字面量收敛：

```bash
rg -n "<event-name>|<command-name>|<field-name>" \
  crates/helix-im crates/helix-driver-host crates/helix-driver-native
rg -n "<event-name>|<field-name>|IpcEventHandler|emit_projection|im:__bus__" \
  /System/Volumes/Data/workspace/angular/visual/cses-client/src/pages/message-v3 \
  /System/Volumes/Data/workspace/angular/visual/cses-client/src-tauri/src/features/helix_im
```

若提供 codebase-memory 图谱，优先 `search_graph` 定位符号，`trace_path` 查入/出调用，`get_code_snippet` 读函数；图谱不能证明运行态值，仍要拿本轮 raw WS、SQLite、EventSink 和 bus 证据。

## 四、最小证据采集

| 证据 | 最低内容 | 不能证明什么 |
|---|---|---|
| raw WS | run/corr、action、seq、完整相关 data | Helix 已消费或持久化 |
| PortReply/Effect | handler、Effect 顺序、PortOutcome、event/data | Go DB/Pulsar 或 Angular 已渲染 |
| SQLite | 本轮 channel/member/message 行，字段和 cursor | EventSink 已发送 |
| Native EventSink | 唯一终态 event、data、关联键 | Tauri listener/Angular 已收到 |
| Tauri bus | `im:__bus__` 原始序列化 payload | Angular store/view model 已合并 |
| Angular | adapter 输出、upsert 后对象、最终 DOM 状态 | Helix/Go durable correctness |

以上证据必须来自同一轮、同一关联键；跨轮拼接只能标为 `UNVERIFIED`。
