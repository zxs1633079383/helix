#![cfg(feature = "sdk-metrics")]

use std::ffi::c_void;
use std::sync::atomic::{AtomicU64, Ordering};

use helix_core::effect::DomainEventBytes;
use helix_core::ports::EventSink;
use helix_driver_ffi::event_sink::FfiEventSink;
use helix_driver_ffi::vtable::CppHostVtable;
use helix_driver_host::metrics::AsyncMetricSink;
use helix_driver_host::BatchSink;

struct CallbackState {
    callbacks: AtomicU64,
    events: AtomicU64,
    bytes: AtomicU64,
    malformed: AtomicU64,
}

unsafe extern "C" fn on_batch(ctx: *mut c_void, ptr: *const u8, len: usize) {
    if ctx.is_null() || ptr.is_null() {
        return;
    }
    // SAFETY: FfiEventSink 借出契约保证 ptr..ptr+len 在回调期间有效；ctx 由 FfiProbe
    // 独占 Box 持有到 sink drop 后，回调期间不会释放。
    let (state, bytes) = unsafe {
        (
            &*(ctx as *const CallbackState),
            std::slice::from_raw_parts(ptr, len),
        )
    };
    match helix_driver_ffi::batch::decode_batch(bytes) {
        Some(events) => {
            state.callbacks.fetch_add(1, Ordering::Relaxed);
            state
                .events
                .fetch_add(events.len() as u64, Ordering::Relaxed);
            state.bytes.fetch_add(len as u64, Ordering::Relaxed);
        }
        None => {
            state.malformed.fetch_add(1, Ordering::Relaxed);
        }
    }
}

pub struct FfiProbe {
    sink: Option<FfiEventSink>,
    state: *mut CallbackState,
}

impl FfiProbe {
    pub fn new(metrics: std::sync::Arc<dyn AsyncMetricSink>) -> Self {
        let state = Box::into_raw(Box::new(CallbackState {
            callbacks: AtomicU64::new(0),
            events: AtomicU64::new(0),
            bytes: AtomicU64::new(0),
            malformed: AtomicU64::new(0),
        }));
        let sink = FfiEventSink::new(CppHostVtable {
            on_domain_events_batch: Some(on_batch),
            now_ms: None,
            ctx: state.cast(),
        })
        .with_metric_sink(metrics);
        Self {
            sink: Some(sink),
            state,
        }
    }

    pub fn forward(&self, event: DomainEventBytes) {
        if let Some(sink) = &self.sink {
            sink.emit(event);
            sink.flush();
        }
    }

    pub fn snapshot(&self) -> (u64, u64, u64, u64) {
        // SAFETY: state 由本对象持有且只在 Drop 回收；读取原子不创建别名可变引用。
        let state = unsafe { &*self.state };
        (
            state.callbacks.load(Ordering::Relaxed),
            state.events.load(Ordering::Relaxed),
            state.bytes.load(Ordering::Relaxed),
            state.malformed.load(Ordering::Relaxed),
        )
    }
}

impl Drop for FfiProbe {
    fn drop(&mut self) {
        drop(self.sink.take());
        // sink 已先释放，后续不会再使用 ctx；Box 只回收一次。
        unsafe { drop(Box::from_raw(self.state)) };
    }
}

// FfiEventSink 本身满足 Send+Sync；ctx 指向仅含原子的 state，跨任务读取安全。
unsafe impl Send for FfiProbe {}
unsafe impl Sync for FfiProbe {}
