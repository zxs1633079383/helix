//! FfiEventSink — 每 step 攒批 → 单次 C++ 回调过边界（HX-C007 核心）。
//!
//! ## 数据流
//!
//! pump 每 step：core 吐多条 `Effect::Emit` → dispatch 逐条调 `EventSink::emit`（此处仅入缓冲）
//! → step 末 pump 调 `BatchSink::flush`（此处编码整批 + 单次 `on_domain_events_batch`）。
//! **绝不逐条过 FFI 边界**——攒批一次过（vs Dioxus sledgehammer 逐 mutation 教训）。
//!
//! ## 缓冲并发
//!
//! `emit(&self)` 是同步 `&self`。pump 在专用单线程跑（current_thread runtime），emit 与 flush
//! 都在该线程串行调用——本无竞争。但 `EventSink: Send + Sync`（native target MaybeSend=Send）
//! 要求结构跨线程安全，故用 `Mutex<Vec<Bytes>>`（无竞争下锁开销可忽略）。

use bytes::Bytes;
use parking_lot::Mutex;
use std::sync::Arc;
use std::time::Instant;

use helix_core::effect::DomainEventBytes;
use helix_core::ports::EventSink;
use helix_driver_host::BatchSink;
use helix_driver_host::{
    AsyncMetricSink, LabelKey, MetricEvent, MetricId, MetricLabels, NoopMetricSink,
};

use crate::batch::encode_batch_into;
use crate::vtable::CppHostVtable;

/// FFI 领域事件 sink：攒本 step 的 Emit，flush 时整批一次回调 C++。
pub struct FfiEventSink {
    /// C++ 回调表（含 `on_domain_events_batch` + ctx）。Copy，整体持有。
    vtable: CppHostVtable,
    /// 本 step 攒下的 DomainEvent 原文缓冲；flush 后 take（容量复用 std::mem::take 后保留在 guard）。
    buffer: Mutex<Vec<Bytes>>,
    /// 编码 scratch buffer（PI-4 稳态零分配）：flush 把整批编码进它，回调返回即可复用——
    /// 借出契约下 buffer 始终 Rust own、回调消费完即空闲，故不每 step 新 Vec。
    scratch: Mutex<Vec<u8>>,
    metrics: Arc<dyn AsyncMetricSink>,
}

impl FfiEventSink {
    pub fn new(vtable: CppHostVtable) -> Self {
        Self {
            vtable,
            buffer: Mutex::new(Vec::new()),
            scratch: Mutex::new(Vec::new()),
            metrics: Arc::new(NoopMetricSink),
        }
    }

    pub fn with_metric_sink(mut self, metrics: Arc<dyn AsyncMetricSink>) -> Self {
        self.metrics = metrics;
        self
    }
}

impl EventSink for FfiEventSink {
    /// 仅入缓冲，不过边界（过边界推迟到 step 末的 flush，HX-C007）。
    fn emit(&self, event: DomainEventBytes) {
        self.buffer.lock().push(event.0);
    }
}

impl BatchSink for FfiEventSink {
    /// step 末：把缓冲的全部 DomainEvent 编码成单 buffer，一次调 `on_domain_events_batch`，清空。
    ///
    /// 空缓冲直接返回（pump 已对空 step 跳过 flush，此处再防御一次，避免空批回调）。
    fn flush(&self) {
        // 先取走缓冲（持锁最短），再编码 + 回调（不持锁，回调可能耗时）。
        let events = {
            let mut guard = self.buffer.lock();
            if guard.is_empty() {
                return;
            }
            std::mem::take(&mut *guard)
        };

        let Some(cb) = self.vtable.on_domain_events_batch else {
            // host 未提供回调 → 事件无出口，丢弃本批 + warn（健壮性兜底，生产应填）。
            tracing::warn!(
                count = events.len(),
                "CppHostVtable.on_domain_events_batch 为 None，丢弃本批领域事件"
            );
            return;
        };

        // scratch 复用（PI-4）：编码进持有的 scratch buffer，回调返回即可复用（借出契约下 buffer
        // 始终 Rust own、回调消费完即空闲 → 稳态零分配，不每 step 新 Vec）。
        let mut scratch = self.scratch.lock();
        encode_batch_into(&events, &mut scratch);
        let callback_started = self.metrics.is_enabled().then(Instant::now);
        // SAFETY: cb 由 host(C++ shim) 经 vtable 提供，契约保证 helix_destroy 返回前有效（vtable.rs
        // ctx 生命周期）。scratch ptr/len 在 cb 回调期间始终有效（持锁存活到回调返回）；**借出契约**：
        // ptr 仅回调期间有效，C++ shim 在回调线程内 memcpy 稳定化吸收 Dart 异步（不逃逸 ptr，不新增
        // free 符号，vtable.rs 头注契约）。ctx 原样回传不解引用。
        unsafe {
            cb(self.vtable.ctx, scratch.as_ptr(), scratch.len());
        }
        if let Some(started) = callback_started {
            let labels = MetricLabels::one(LabelKey::Stage, "ffi");
            let _ = self.metrics.try_record(MetricEvent::histogram(
                MetricId::FfiBatchEvents,
                events.len() as f64,
                labels,
            ));
            let _ = self.metrics.try_record(MetricEvent::histogram(
                MetricId::FfiBatchBytes,
                scratch.len() as f64,
                labels,
            ));
            let _ = self.metrics.try_record(MetricEvent::histogram(
                MetricId::FfiCallbackDurationSeconds,
                started.elapsed().as_secs_f64(),
                labels,
            ));
            let _ = self.metrics.try_record(MetricEvent::counter(
                MetricId::OperationsTotal,
                events.len() as f64,
                labels
                    .with(LabelKey::Operation, "callback")
                    .with(LabelKey::Status, "ok"),
            ));
        }
        // scratch 保留容量供下个 step 复用（lock 在此释放；buffer 不 drop）。
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use helix_driver_host::RecordOutcome;
    use std::ffi::c_void;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[derive(Default)]
    struct RecordingMetrics(Mutex<Vec<MetricEvent>>);

    impl AsyncMetricSink for RecordingMetrics {
        fn try_record(&self, event: MetricEvent) -> RecordOutcome {
            self.0.lock().push(event);
            RecordOutcome::Accepted
        }
    }

    unsafe extern "C" fn capture_batch(ctx: *mut c_void, _ptr: *const u8, len: usize) {
        // SAFETY: 测试在 flush 调用期间保持 AtomicUsize 存活，ctx 正是其有效指针。
        let captured = unsafe { &*(ctx.cast::<AtomicUsize>()) };
        captured.store(len, Ordering::Relaxed);
    }

    #[test]
    fn flush_records_batch_events_bytes_and_callback_duration() {
        let captured = AtomicUsize::new(0);
        let vtable = CppHostVtable {
            on_domain_events_batch: Some(capture_batch),
            now_ms: None,
            ctx: (&captured as *const AtomicUsize).cast_mut().cast(),
        };
        let metrics = Arc::new(RecordingMetrics::default());
        let sink = FfiEventSink::new(vtable).with_metric_sink(metrics.clone());
        sink.emit(DomainEventBytes(Bytes::from_static(b"a")));
        sink.emit(DomainEventBytes(Bytes::from_static(b"bc")));
        sink.flush();

        assert!(captured.load(Ordering::Relaxed) > 0);
        let events = metrics.0.lock();
        assert_eq!(events.len(), 4);
        assert_eq!(events[0].id, MetricId::FfiBatchEvents);
        assert_eq!(events[0].value, 2.0);
        assert_eq!(events[1].id, MetricId::FfiBatchBytes);
        assert_eq!(events[2].id, MetricId::FfiCallbackDurationSeconds);
        assert_eq!(events[3].id, MetricId::OperationsTotal);
        assert_eq!(events[3].value, 2.0);
    }
}
