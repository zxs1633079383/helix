//! NativeClock — SystemTime

use helix_core::ports::Clock;

#[derive(Clone, Debug)]
pub struct NativeClock;

impl Clock for NativeClock {
    fn now_ms(&self) -> u64 {
        use std::time::{SystemTime, UNIX_EPOCH};
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64
    }
}
