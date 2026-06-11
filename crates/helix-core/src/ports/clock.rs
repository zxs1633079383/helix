//! Clock port — 时间注入抽象
//!
//! ## 为什么必须注入（不能用 std::time）
//!
//! - `std::time::Instant` 在 wasm32-unknown-unknown **运行时 panic**（非编译期）
//! - 确定性回放和单元测试需要手动控制时间（MockClock）
//!
//! ## 三端实现
//! - native: SystemTime
//! - wasm:   js_sys::Date::now()
//! - ffi:    host fn() -> u64 回调

use crate::platform::MaybeSend;

/// 时间源的 core 抽象。
///
/// 返回 Unix epoch 毫秒数。
/// 注意：仅提供墙钟时间，不提供单调时钟（Instant）。
/// 超时/退避的时间差计算用两次 now_ms() 之差，精度足够 IM 场景。
pub trait Clock: Clone + MaybeSend + 'static {
    /// Unix epoch 毫秒数（墙钟时间）
    fn now_ms(&self) -> u64;
}

// ─── MockClock（确定性测试用）────────────────────────────────────────────────

/// 可手动控制时间的 Mock Clock，用于确定性单元测试和属性测试。
///
/// ## MIN-7 修复
///
/// 原实现使用 `Arc<AtomicU64>`，但 wasm32-unknown-unknown 在没有
/// `target-feature=+atomics` 时不支持原子操作（链接时报错）。
/// MockClock 仅在单线程测试上下文使用，不需要原子语义。
/// 改为 `std::cell::Cell<u64>`（单线程可变，内部可变性），
/// 并移到 `#[cfg(any(test, feature = "testing"))]` 门控下，
/// 确保生产构建不依赖 atomics。
///
/// 若需要多线程共享 MockClock（如异步测试的多任务），
/// 请自行包裹 `Arc<std::sync::Mutex<u64>>`——但这在测试中也很罕见。
///
/// ## 示例
///
/// ```rust
/// use helix_core::ports::MockClock;
/// use helix_core::ports::clock::Clock;
/// let clock = MockClock::new(1_000_000);
/// assert_eq!(clock.now_ms(), 1_000_000);
/// clock.advance(15_001);  // 模拟 15s 超时
/// assert_eq!(clock.now_ms(), 1_015_001);
/// ```
#[cfg(any(test, feature = "testing"))]
#[derive(Debug, Default)]
pub struct MockClock {
    inner: std::cell::Cell<u64>,
}

#[cfg(any(test, feature = "testing"))]
impl MockClock {
    /// 创建初始时间为 `initial_ms` 的 MockClock
    pub fn new(initial_ms: u64) -> Self {
        Self {
            inner: std::cell::Cell::new(initial_ms),
        }
    }

    /// 推进时间 `delta_ms` 毫秒（用于模拟超时、门控等）
    pub fn advance(&self, delta_ms: u64) {
        self.inner.set(self.inner.get() + delta_ms);
    }

    /// 直接设置时间
    pub fn set(&self, ms: u64) {
        self.inner.set(ms);
    }
}

#[cfg(any(test, feature = "testing"))]
impl Clone for MockClock {
    fn clone(&self) -> Self {
        Self { inner: std::cell::Cell::new(self.inner.get()) }
    }
}

#[cfg(any(test, feature = "testing"))]
impl Clock for MockClock {
    fn now_ms(&self) -> u64 {
        self.inner.get()
    }
}
