//! Browser one-shot timer registry with generation-safe cancellation.

use std::collections::HashMap;

use helix_core::effect::TimerId;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimerToken {
    pub id: TimerId,
    generation: u64,
}

/// Runtime-independent timer identity book used by the browser adapter and tests.
#[derive(Debug, Default)]
pub struct TimerBook {
    active: HashMap<TimerId, u64>,
    next_generation: u64,
}

impl TimerBook {
    pub fn schedule(&mut self, id: TimerId) -> TimerToken {
        self.next_generation = self.next_generation.wrapping_add(1).max(1);
        let token = TimerToken {
            id,
            generation: self.next_generation,
        };
        self.active.insert(id, token.generation);
        token
    }

    pub fn cancel(&mut self, id: TimerId) -> bool {
        self.active.remove(&id).is_some()
    }

    pub fn fire(&mut self, token: TimerToken) -> Option<TimerId> {
        if self.active.get(&token.id) != Some(&token.generation) {
            return None;
        }
        self.active.remove(&token.id);
        Some(token.id)
    }

    pub fn len(&self) -> usize {
        self.active.len()
    }

    pub fn is_empty(&self) -> bool {
        self.active.is_empty()
    }
}

#[cfg(target_arch = "wasm32")]
#[derive(Debug, Clone, Copy, Default)]
pub struct BrowserClock;

#[cfg(target_arch = "wasm32")]
impl helix_core::ports::Clock for BrowserClock {
    fn now_ms(&self) -> u64 {
        js_sys::Date::now().max(0.0) as u64
    }
}

#[cfg(target_arch = "wasm32")]
pub struct BrowserTimerRegistry {
    book: TimerBook,
    handles: HashMap<TimerId, gloo_timers::callback::Timeout>,
}

#[cfg(target_arch = "wasm32")]
impl BrowserTimerRegistry {
    pub fn new() -> Self {
        Self {
            book: TimerBook::default(),
            handles: HashMap::new(),
        }
    }

    pub fn schedule(
        &mut self,
        id: TimerId,
        after_ms: u64,
        on_fire: impl FnOnce(TimerToken) + 'static,
    ) {
        let token = self.book.schedule(id);
        // Browser setTimeout uses a signed 32-bit millisecond delay. Larger
        // values can wrap negative and fire immediately in JavaScript engines.
        let delay = after_ms.min(i32::MAX as u64) as u32;
        let handle = gloo_timers::callback::Timeout::new(delay, move || on_fire(token));
        self.handles.insert(id, handle);
    }

    pub fn cancel(&mut self, id: TimerId) {
        self.book.cancel(id);
        self.handles.remove(&id);
    }

    /// Called by the timeout callback before the Timer tick is enqueued.
    pub fn mark_fired(&mut self, token: TimerToken) -> Option<TimerId> {
        let id = self.book.fire(token)?;
        self.handles.remove(&id);
        Some(id)
    }

    pub fn clear(&mut self) {
        self.book = TimerBook::default();
        self.handles.clear();
    }
}

#[cfg(target_arch = "wasm32")]
impl Default for BrowserTimerRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reschedule_invalidates_old_callback_and_fire_is_one_shot() {
        let mut book = TimerBook::default();
        let id = TimerId::from_raw(7);
        let old = book.schedule(id);
        let current = book.schedule(id);
        assert_eq!(book.fire(old), None);
        assert_eq!(book.fire(current), Some(id));
        assert_eq!(book.fire(current), None);
        assert!(book.is_empty());
    }

    #[test]
    fn cancel_is_idempotent() {
        let mut book = TimerBook::default();
        let id = TimerId::from_raw(9);
        let token = book.schedule(id);
        assert!(book.cancel(id));
        assert!(!book.cancel(id));
        assert_eq!(book.fire(token), None);
    }
}
