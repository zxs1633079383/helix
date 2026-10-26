//! EventSink port — 领域事件发布抽象
//!
//! ## 设计说明
//!
//! - **同步 emit**（不 async）：EventSink 只是"登记意图"，不等待订阅者处理完成
//! - driver 实现可以把事件放入 broadcast channel，实际分发在 driver 内部异步进行
//! - helix-im 的 `ImDomainEvent` 在 ACL-1 序列化为 DomainEventBytes 再 emit，
//!   core 永远看不到 IM 的 enum 类型
//!
//! ## 三端实现
//! - native: tauri AppHandle.emit（Tauri 的 IPC 事件总线）
//! - wasm:   wasm-bindgen JS callback
//! - ffi:    C ABI on_event_fn 回调

use crate::effect::DomainEventBytes;
use crate::platform::{MaybeSend, MaybeSync};

/// 领域事件发布的 core 抽象。
///
/// emit 是同步的：调用返回即表示事件已"入队"，实际分发由 driver 异步处理。
pub trait EventSink: MaybeSend + MaybeSync + 'static {
    /// 发布一个已序列化的领域事件。
    ///
    /// 实现必须是非阻塞的（不应 block 等待订阅者）。
    fn emit(&self, event: DomainEventBytes);
}
