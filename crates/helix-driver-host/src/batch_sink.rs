use helix_core::ports::EventSink;

/// 每 step 批处理钩子（host 层扩展，不污染 core 的 [`EventSink`]）。
///
/// ## 为什么钩子在 host 不在 core
///
/// core 的 `EventSink::emit` 是单条同步「登记意图」契约（sans-IO，HX-C001：core 零运行时）。
/// 「一个 step 产出的 Emit 整批跨边界一次过」是**驱动侧 egress 批处理**（HX-C007），属 driver
/// 关注点。把 flush 钩子放 core 会让 core 认识「批边界」这个运行时概念 → 越界。故在此 host 层
/// 用 supertrait 扩展：pump 在**每次 `dispatch_effects` 之后**调一次 `flush()`，让需要批 egress
/// 的 sink（FFI vtable）攒批后一次性过 FFI；不需要的（native broadcast 即时发）用默认 no-op。
///
/// ## 不变量
///
/// - `flush()` 调用点 = 每个 step 的 effects 全部 dispatch 完成之后（含初始 `start()` 那批）。
/// - 默认实现 no-op：native `impl BatchSink for NativeEventSink {}` 行为零变。
/// - flush 内不得 panic（FFI 实现跨边界回调，遵 HX-C006：生产 cdylib panic=abort）。
pub trait BatchSink: EventSink {
    /// 一个 step 的全部 Emit 已 dispatch 完成 → 把本 step 攒下的批整体 egress。
    ///
    /// 默认 no-op（即时发的 sink 无需批，如 native broadcast）。
    fn flush(&self) {}
}
