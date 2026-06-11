//! JsClock — js_sys::Date::now()
//!
//! ⚠️ 不可用 std::time::Instant（wasm32-unknown-unknown 运行时 panic）
//! ⚠️ 不可用 std::time::SystemTime（不稳定）
//! 必须用 js_sys::Date::now() 或注入的 Clock port

use helix_core::ports::Clock;
use helix_core::platform::MaybeSend;

#[derive(Clone, Debug)]
pub struct JsClock;

impl Clock for JsClock {
    fn now_ms(&self) -> u64 {
        #[cfg(target_arch = "wasm32")]
        {
            js_sys::Date::now() as u64
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            // 仅用于非 wasm 目标的编译检查，运行时不应到达此处
            0
        }
    }
}
