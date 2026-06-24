//! instrument — 外部宿主在**投影路径**插录放/日志装饰器的非侵入范例。
//!
//! ## 为什么要这个范例
//!
//! 外部组装根（如 loopforge-tauri-im 内嵌 helix 引擎作 Tauri 后端）想用装饰器
//! `Recording<P>` 包住各 port 抓「输入/输出对」做日志/录放对账。其余 port（Storage /
//! HttpRequester / Transport / Clock）都是 core 的 trait，直接 `impl CoreTrait for
//! Recording<Inner>` 即可。**唯独投影面有一道坎**：泛型事件泵 [`crate::engine::EngineDeps`]
//! 的 `E` 与 [`crate::engine::run_engine_loop`] 的 `E` bound 的是 **host 本地的
//! [`BatchSink`]**（`EventSink` 的 supertrait，多一个 `flush()` 批边界钩子），不是 core 的
//! `EventSink`。所以**只实现 `EventSink` 的装饰器装不进引擎**——必须同时实现 `BatchSink`。
//!
//! 本模块给一个**最小、可编译、零依赖业务**的 [`RecordingSink`]：把任意 `S: BatchSink` 包一层，
//! 每条 `emit` 先喂给注入的回调（录放/计数/日志）再透传给内层；`flush` 原样透传（保 HX-C007
//! 批边界语义不变）。外部宿主可直接抄此形态，把回调换成自己的录放器。
//!
//! ## 不改既有逻辑
//!
//! 这是 **add-only** helper：不碰 `engine.rs` / dispatch / 任何 pump 核。引擎照旧只认
//! `E: BatchSink`；`RecordingSink<S>` 因为 `S: BatchSink` 且自身 `impl BatchSink` 而满足 bound，
//! 像普通 sink 一样塞进 `EngineDeps.event_sink` 即可。

use std::sync::Arc;

use helix_core::effect::DomainEventBytes;
use helix_core::ports::EventSink;

use crate::engine::BatchSink;

/// 投影面录放/观测装饰器：包住任意 [`BatchSink`]，每条 emit 旁路给 `observe` 再透传。
///
/// ## 类型参数
/// - `S`: 被包裹的真实 sink（如 native 的 `NativeEventSink` / ffi 的 `FfiEventSink`）。
///
/// ## 关键：为什么实现 `BatchSink` 而非只实现 `EventSink`
///
/// 见模块头注——引擎的 `E` bound 是 `BatchSink`。若只 `impl EventSink for RecordingSink`，
/// `RecordingSink` 不满足 `E: BatchSink`，**编译期就装不进 [`crate::engine::EngineDeps`]**。
/// 故下方两个 impl 缺一不可：`EventSink`（旁路 + 透传 emit）+ `BatchSink`（透传 flush）。
///
/// ## emit 顺序约定
///
/// 先调 `observe(&event)` 再 `inner.emit(event)`：录放器先看到事件，再让真实 sink 发出。
/// `observe` 必须**非阻塞、不 panic**（与 `EventSink::emit` 契约一致；FFI 场景 flush 跨边界
/// 回调，panic 会越界，HX-C006）。
pub struct RecordingSink<S> {
    inner: S,
    /// 每条 emit 的旁路观测回调（录放 / 计数 / 日志）。`Arc` 便于与外部录放器共享句柄。
    observe: Arc<dyn Fn(&DomainEventBytes) + Send + Sync>,
}

impl<S> RecordingSink<S> {
    /// 包裹 `inner`，注入旁路观测回调 `observe`。
    pub fn new(inner: S, observe: Arc<dyn Fn(&DomainEventBytes) + Send + Sync>) -> Self {
        Self { inner, observe }
    }

    /// 取回内层 sink（卸下装饰器）。
    pub fn into_inner(self) -> S {
        self.inner
    }
}

impl<S: EventSink> EventSink for RecordingSink<S> {
    /// 旁路录放 + 透传：先让 `observe` 看到事件，再交给内层 sink 真正 emit。
    fn emit(&self, event: DomainEventBytes) {
        (self.observe)(&event);
        self.inner.emit(event);
    }
}

impl<S: BatchSink> BatchSink for RecordingSink<S> {
    /// 透传批边界 flush（保 HX-C007 语义不变）。
    ///
    /// native 内层 no-op → 整体 no-op；ffi 内层攒批过 vtable → 整体仍每 step 一次过边界。
    /// 装饰器**不**改批边界本身——录放只发生在 `emit` 旁路，不干预 flush 攒批/出口策略。
    fn flush(&self) {
        self.inner.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// 内层 spy：记 emit 调用次数 + flush 调用次数，验证装饰器透传无丢。
    #[derive(Default)]
    struct SpySink {
        emits: AtomicUsize,
        flushes: AtomicUsize,
    }
    impl EventSink for SpySink {
        fn emit(&self, _event: DomainEventBytes) {
            self.emits.fetch_add(1, Ordering::Relaxed);
        }
    }
    impl BatchSink for SpySink {
        fn flush(&self) {
            self.flushes.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// 回归锚：RecordingSink 旁路观测每条 emit + 透传给内层（不丢、不改顺序计数）。
    #[test]
    fn recording_sink_observes_and_forwards_emit() {
        let observed = Arc::new(AtomicUsize::new(0));
        let observed_clone = Arc::clone(&observed);
        let inner = SpySink::default();

        // 注：测试里 into_inner 取回 inner 验证内层计数，故先单独持 inner 不可——
        // 改为把内层包进装饰器后只通过装饰器 emit，再 into_inner 取回查内层计数。
        let sink = RecordingSink::new(
            inner,
            Arc::new(move |_ev: &DomainEventBytes| {
                observed_clone.fetch_add(1, Ordering::Relaxed);
            }),
        );

        sink.emit(DomainEventBytes(Bytes::from_static(b"a")));
        sink.emit(DomainEventBytes(Bytes::from_static(b"b")));
        sink.flush();

        // 旁路观测看到 2 条
        assert_eq!(
            observed.load(Ordering::Relaxed),
            2,
            "observe 应看到每条 emit"
        );

        // 透传给内层：内层 emit 2 次 + flush 1 次
        let inner = sink.into_inner();
        assert_eq!(
            inner.emits.load(Ordering::Relaxed),
            2,
            "内层应收到全部 emit"
        );
        assert_eq!(inner.flushes.load(Ordering::Relaxed), 1, "flush 应透传内层");
    }
}
