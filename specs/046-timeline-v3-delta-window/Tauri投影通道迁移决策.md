# Tauri 投影通道迁移决策

## 结论
PC/Tauri 将 `im:interaction:frame` 与 `im:sync:gap` 从 `im:__bus__` Event 迁移到同一条、每个
WebView 独立订阅的 `tauri::ipc::Channel`。两类消息共享顺序域；不得让 Frame 走 Channel、
Gap 继续走 Event。`im:ready`、`im:health`、窗口生命周期和低频诊断等小型广播继续使用 Event。

## 当前事实
- Angular 到 Rust 的真实业务命令已经使用 `invoke` IPC，包括发送、撤回、分页、定位、
  snapshot/resync 与 `__helix_frame_applied`。
- Rust 到 Angular 的 Helix projection 当前统一通过 `app.emit` 或 `app.emit_str("im:__bus__")`
  下发；InteractionFrame 也未使用 `tauri::ipc::Channel`。
- 当前 candidate 中 WDIO 只传 fixture 身份，约 121 KiB 的 canonical Frame 由 Rust 读取并经
  生产 `emit_str` 路径发送，因此该阻塞不能归类为 WDIO 大参数限制。
- Tauri 官方将 Event 定位为小量、低吞吐通知；Event payload 始终为 JSON 字符串，较大或要求
  有序的数据流应使用 Channel。官方未声明 121 KiB 的固定硬上限。

## 消息合同
统一下行消息使用 tagged union：`Frame { generation, transportSeq, frame }` 与
`SyncGap { generation, transportSeq, dropped }`。`generation` 在每次 WebView 重新订阅时递增，
`transportSeq` 在同一 generation 内严格递增。一个 InteractionFrame 必须作为一条完整 Channel
message 发送，不得拆分后由 Angular 拼装，以保留 Persist barrier、revision 和原子 apply 语义。

## 生命周期与恢复
Angular 必须先安装 `channel.onmessage`，再调用 `subscribe_im_projection`；invoke 成功后才允许
attach surface 或请求首屏。Rust 按 `windowLabel -> subscription` 管理 sender，窗口刷新、关闭或
重新登录时替换旧订阅。Channel 断开、generation 改变、transportSeq 不连续、SyncGap 或 surface
revision gap 任一发生时，当前窗口标记为 dirty，并通过 invoke 请求 authoritative snapshot。
`channel.send()` 成功不等于 UI 已应用，现有 `FrameApplied`、ack timeout 和 snapshot fallback
必须保留。

## 收益与代价
收益是 Frame/Gap 全序、较大 Timeline Frame 使用官方推荐通道、避免向无关窗口广播大 payload、
减少多层 Event envelope，并获得明确的 listener-ready 握手。代价是增加每窗口订阅注册表、重订阅
和清理逻辑；Channel 不持久、不自动提供业务 ack，也不会自动压缩 JSON 或消除跨边界内存复制。

## 迁移与 Gate
迁移期只切换 InteractionFrame 与 SyncGap，MessageV3 和其他小事件继续使用 `im:__bus__`，不得把
双写 Event+Channel 当作长期方案。验收必须覆盖 32/64/96/121/192/256 KiB 大小阶梯，以及 Rust
send、Angular Channel callback、SurfaceRuntime outcome、FrameApplied、DOM 五跳；另测同一
generation 内 Frame/Gap 顺序、断线重订阅、多窗口隔离、duplicate/revision gap 和 snapshot 恢复。
Mock、send 返回 Ok 或单独 DOM 截图均不能证明生产链 Green。

## 参考
- Tauri Calling the Frontend from Rust：https://v2.tauri.app/develop/calling-frontend/
- 当前 PC sink：`cses-client/src-tauri/src/features/helix_im/event_sink.rs`
- 当前 PC bridge：`cses-client/src-tauri/src/features/helix_im/runtime.rs`
- 当前 Angular bus：`cses-client/src/app/core/services/tauri/im-event-bus.ts`
