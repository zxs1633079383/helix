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
//! 都由该线程调用；上传 observer 可由 I/O worker 并发入队。业务、诊断各自持锁，flush 用
//! scratch 锁串行化并借出回调 buffer；observer 不调用 C++，不持有 callback buffer。

use bytes::Bytes;
use parking_lot::Mutex;
use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Instant;

use helix_core::effect::DomainEventBytes;
use helix_core::ports::EventSink;
use helix_driver_host::upload::{UploadResponseObservation, UploadResponseObserver};
use helix_driver_host::BatchSink;
use helix_driver_host::{
    AsyncMetricSink, LabelKey, MetricEvent, MetricId, MetricLabels, NoopMetricSink,
};

use crate::batch::encode_batch_into;
use crate::vtable::CppHostVtable;

const UPLOAD_DIAGNOSTIC_CAPACITY: usize = 128;
const UPLOAD_DIAGNOSTIC_MAX_BYTES: usize = 1024;
const UPLOAD_DIAGNOSTIC_EVENT: &str = "helix:diagnostic:upload-response";

#[derive(Default)]
struct UploadDiagnostics {
    queue: VecDeque<(u64, Bytes)>,
    sequence: u64,
    dropped_count: u64,
    reported_dropped_count: u64,
    last_dropped_sequence: u64,
}

impl UploadDiagnostics {
    fn has_pending(&self) -> bool {
        !self.queue.is_empty() || self.dropped_count != self.reported_dropped_count
    }

    fn drop_current(&mut self) {
        self.dropped_count = self.dropped_count.saturating_add(1);
        self.last_dropped_sequence = self.sequence;
    }

    fn drain_into(&mut self, events: &mut Vec<Bytes>) {
        let mut pending_loss = None;
        if self.dropped_count != self.reported_dropped_count {
            // Loss is a queue fact, never an invented HTTP response or business terminal.
            let loss = serde_json::json!({
                "event": UPLOAD_DIAGNOSTIC_EVENT,
                "data": {
                    "version": 1, "kind": "loss", "method": "PUT",
                    "httpStatus": null, "objectKey": null, "fileSize": null, "durationUs": null,
                    "sequence": self.last_dropped_sequence, "droppedCount": self.dropped_count
                }
            });
            if let Ok(bytes) = serde_json::to_vec(&loss) {
                pending_loss = Some(Bytes::from(bytes));
                self.reported_dropped_count = self.dropped_count;
            }
        }
        // Merge the aggregate loss at its last dropped attempt; accepted bytes stay unchanged.
        for (sequence, bytes) in self.queue.drain(..) {
            if sequence > self.last_dropped_sequence {
                if let Some(loss) = pending_loss.take() {
                    events.push(loss);
                }
            }
            events.push(bytes);
        }
        if let Some(loss) = pending_loss {
            events.push(loss);
        }
    }
}

/// FFI 领域事件 sink：攒本 step 的 Emit，flush 时整批一次回调 C++。
pub struct FfiEventSink {
    /// C++ 回调表（含 `on_domain_events_batch` + ctx）。Copy，整体持有。
    vtable: CppHostVtable,
    /// 本 step 攒下的 DomainEvent 原文缓冲；flush 后 take（容量复用 std::mem::take 后保留在 guard）。
    buffer: Mutex<Vec<Bytes>>,
    /// 编码 scratch buffer（PI-4 稳态零分配）：flush 把整批编码进它，回调返回即可复用——
    /// 借出契约下 buffer 始终 Rust own、回调消费完即空闲，故不每 step 新 Vec。
    scratch: Mutex<Vec<u8>>,
    /// Dedicated bounded diagnostics; business Emit bytes remain in buffer unchanged.
    upload_diagnostics: Mutex<UploadDiagnostics>,
    metrics: Arc<dyn AsyncMetricSink>,
}

impl FfiEventSink {
    pub fn new(vtable: CppHostVtable) -> Self {
        Self {
            vtable,
            buffer: Mutex::new(Vec::new()),
            scratch: Mutex::new(Vec::new()),
            upload_diagnostics: Mutex::new(UploadDiagnostics::default()),
            metrics: Arc::new(NoopMetricSink),
        }
    }

    pub fn with_metric_sink(mut self, metrics: Arc<dyn AsyncMetricSink>) -> Self {
        self.metrics = metrics;
        self
    }
}

impl UploadResponseObserver for FfiEventSink {
    fn observe(&self, observation: UploadResponseObservation<'_>) {
        let mut diagnostics = self.upload_diagnostics.lock();
        diagnostics.sequence = diagnostics.sequence.saturating_add(1);
        // Bound the borrowed key before serialization can copy/escape it.
        if diagnostics.queue.len() == UPLOAD_DIAGNOSTIC_CAPACITY
            || observation.object_key.len() > UPLOAD_DIAGNOSTIC_MAX_BYTES
        {
            diagnostics.drop_current();
            return;
        }
        let envelope = serde_json::json!({
            "event": UPLOAD_DIAGNOSTIC_EVENT,
            "data": {
                "version": 1, "kind": "response", "method": observation.method,
                "httpStatus": observation.http_status, "objectKey": observation.object_key,
                "fileSize": observation.file_size, "durationUs": observation.duration_us,
                "sequence": diagnostics.sequence, "droppedCount": diagnostics.dropped_count
            }
        });
        match serde_json::to_vec(&envelope) {
            Ok(bytes) if bytes.len() <= UPLOAD_DIAGNOSTIC_MAX_BYTES => {
                let sequence = diagnostics.sequence;
                diagnostics.queue.push_back((sequence, Bytes::from(bytes)));
            }
            _ => diagnostics.drop_current(),
        }
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
    /// Empty business and diagnostic buffers return without a callback.
    fn flush(&self) {
        // Serialize flushes through scratch; producers only lock their separate queues.
        let mut scratch = self.scratch.lock();
        let mut events = std::mem::take(&mut *self.buffer.lock());
        let has_diagnostics = self.upload_diagnostics.lock().has_pending();
        if events.is_empty() && !has_diagnostics {
            return;
        }

        let Some(cb) = self.vtable.on_domain_events_batch else {
            // host 未提供回调 → 事件无出口，丢弃本批 + warn（健壮性兜底，生产应填）。
            if self.metrics.is_enabled() {
                let labels = MetricLabels::one(LabelKey::Stage, "ffi");
                let _ = self.metrics.try_record(MetricEvent::counter(
                    MetricId::FfiCallbackErrorsTotal,
                    1.0,
                    labels,
                ));
                let _ = self.metrics.try_record(MetricEvent::counter(
                    MetricId::FfiEventsDroppedTotal,
                    events.len() as f64,
                    labels,
                ));
            }
            tracing::warn!(
                count = events.len(),
                diagnostics_retained = has_diagnostics,
                "CppHostVtable.on_domain_events_batch 为 None，丢弃领域事件；保留有界上传诊断"
            );
            return;
        };

        // scratch 复用（PI-4）：编码进持有的 scratch buffer，回调返回即可复用（借出契约下 buffer
        // 始终 Rust own、回调消费完即空闲 → 稳态零分配，不每 step 新 Vec）。
        self.upload_diagnostics.lock().drain_into(&mut events);
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
                MetricId::FfiBoundaryBytes,
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
        assert_eq!(events.len(), 5);
        assert_eq!(events[0].id, MetricId::FfiBatchEvents);
        assert_eq!(events[0].value, 2.0);
        assert_eq!(events[1].id, MetricId::FfiBatchBytes);
        assert_eq!(events[2].id, MetricId::FfiBoundaryBytes);
        assert_eq!(events[3].id, MetricId::FfiCallbackDurationSeconds);
        assert_eq!(events[4].id, MetricId::OperationsTotal);
        assert_eq!(events[4].value, 2.0);
    }

    /// 缺失 C++ 回调必须显式记录 callback error 与整批 dropped 事件。
    #[test]
    fn missing_callback_records_dropped_batch() {
        let metrics = Arc::new(RecordingMetrics::default());
        let vtable = CppHostVtable {
            on_domain_events_batch: None,
            now_ms: None,
            ctx: std::ptr::null_mut(),
        };
        let sink = FfiEventSink::new(vtable).with_metric_sink(metrics.clone());
        sink.emit(DomainEventBytes(Bytes::from_static(b"a")));
        sink.emit(DomainEventBytes(Bytes::from_static(b"b")));
        sink.flush();

        let events = metrics.0.lock();
        assert!(events
            .iter()
            .any(|event| event.id == MetricId::FfiCallbackErrorsTotal));
        assert!(events
            .iter()
            .any(|event| { event.id == MetricId::FfiEventsDroppedTotal && event.value == 2.0 }));
    }

    #[derive(Default)]
    struct BatchRecorder {
        batches: Mutex<Vec<Vec<u8>>>,
        callback_threads: Mutex<Vec<std::thread::ThreadId>>,
    }

    unsafe extern "C" fn copy_batch(ctx: *mut c_void, ptr: *const u8, len: usize) {
        // SAFETY: each test keeps BatchRecorder alive until all producers/flushes have joined;
        // FfiEventSink lends ptr/len for this callback, which copies before returning.
        let recorder = unsafe { &*ctx.cast::<BatchRecorder>() };
        // SAFETY: ptr/len are the live sink scratch slice for this callback only.
        let bytes = unsafe { std::slice::from_raw_parts(ptr, len) };
        recorder.batches.lock().push(bytes.to_vec());
        recorder
            .callback_threads
            .lock()
            .push(std::thread::current().id());
    }

    fn recording_sink(recorder: &BatchRecorder) -> FfiEventSink {
        FfiEventSink::new(CppHostVtable {
            on_domain_events_batch: Some(copy_batch),
            now_ms: None,
            ctx: (recorder as *const BatchRecorder).cast_mut().cast(),
        })
    }

    fn observe(sink: &FfiEventSink, object_key: &str) {
        sink.observe(UploadResponseObservation {
            method: "PUT",
            http_status: Some(503),
            object_key,
            file_size: 130_037,
            duration_us: 1234,
        });
    }

    fn diagnostic_values(recorder: &BatchRecorder) -> Vec<serde_json::Value> {
        recorder
            .batches
            .lock()
            .iter()
            .flat_map(|batch| {
                crate::batch::decode_batch(batch)
                    .unwrap()
                    .into_iter()
                    .map(|event| serde_json::from_slice(event).unwrap())
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    #[test]
    fn upload_diagnostic_business_bytes_remain_exact_and_flush_once() {
        let recorder = BatchRecorder::default();
        let sink = recording_sink(&recorder);
        sink.flush();
        assert!(recorder.batches.lock().is_empty());
        let business = [
            b"\x00\xffnot-json".as_slice(),
            b"{ \"event\" : \"business\" }".as_slice(),
        ];
        for bytes in business {
            sink.emit(DomainEventBytes(Bytes::copy_from_slice(bytes)));
        }
        observe(&sink, "actual/request-key.bin");
        assert_eq!(
            sink.buffer.lock().len(),
            2,
            "diagnostics must not masquerade as business Emit"
        );
        assert!(recorder.batches.lock().is_empty(), "observer only enqueues");
        sink.flush();
        sink.flush();
        let batches = recorder.batches.lock();
        assert_eq!(batches.len(), 1);
        let events = crate::batch::decode_batch(&batches[0]).unwrap();
        assert_eq!(events.len(), 3);
        assert_eq!(events[0], business[0]);
        assert_eq!(events[1], business[1]);
        let value: serde_json::Value = serde_json::from_slice(events[2]).unwrap();
        assert_eq!(
            value,
            serde_json::json!({
                "event": "helix:diagnostic:upload-response",
                "data": { "version": 1, "kind": "response", "method": "PUT", "httpStatus": 503,
                    "objectKey": "actual/request-key.bin", "fileSize": 130037, "durationUs": 1234,
                    "sequence": 1, "droppedCount": 0 }
            })
        ); // Exact allowlist excludes URL, headers, cookies, local path, and body.
        assert!(events[2].len() <= UPLOAD_DIAGNOSTIC_MAX_BYTES);
    }

    #[test]
    fn upload_diagnostic_capacity_loss_is_visible_and_cumulative() {
        let recorder = BatchRecorder::default();
        let sink = recording_sink(&recorder);
        for _ in 0..UPLOAD_DIAGNOSTIC_CAPACITY + 2 {
            observe(&sink, "key");
        }
        {
            let diagnostics = sink.upload_diagnostics.lock();
            assert_eq!(diagnostics.queue.len(), 128);
            assert!(diagnostics
                .queue
                .iter()
                .all(|(_, bytes)| bytes.len() <= 1024));
            assert_eq!(diagnostics.sequence, 130);
            assert_eq!(diagnostics.dropped_count, 2);
        }
        sink.flush();
        let values = diagnostic_values(&recorder);
        assert_eq!(values.len(), 129);
        assert_eq!(
            values[128]["data"],
            serde_json::json!({
                "version": 1, "kind": "loss", "method": "PUT", "httpStatus": null,
                "objectKey": null, "fileSize": null, "durationUs": null,
                "sequence": 130, "droppedCount": 2
            })
        );
        observe(&sink, "after-capacity-loss");
        sink.flush();
        sink.flush();
        let values = diagnostic_values(&recorder);
        assert_eq!(values.len(), 130);
        assert_eq!(values[129]["data"]["sequence"], 131);
        assert_eq!(values[129]["data"]["droppedCount"], 2);
        assert_eq!(recorder.batches.lock().len(), 2);
    }

    #[test]
    fn upload_diagnostic_oversize_and_escape_expansion_report_only_loss() {
        let recorder = BatchRecorder::default();
        let sink = recording_sink(&recorder);
        observe(&sink, &"k".repeat(1025)); // Reject borrowed key before copying it.
        observe(&sink, &"\0".repeat(900)); // Fits raw-key limit, exceeds encoded envelope limit.
        assert!(sink.upload_diagnostics.lock().queue.is_empty());
        sink.flush();
        let values = diagnostic_values(&recorder);
        assert_eq!(values.len(), 1);
        assert_eq!(values[0]["data"]["kind"], "loss");
        assert_eq!(values[0]["data"]["sequence"], 2);
        assert_eq!(values[0]["data"]["droppedCount"], 2);
        for fact in ["httpStatus", "objectKey", "fileSize", "durationUs"] {
            assert!(values[0]["data"][fact].is_null());
        }
        assert!(crate::batch::decode_batch(&recorder.batches.lock()[0]).unwrap()[0].len() <= 1024);
    }

    #[test]
    fn upload_diagnostic_loss_and_responses_deliver_in_attempt_sequence_order() {
        let recorder = BatchRecorder::default();
        let sink = recording_sink(&recorder);
        let oversize = "k".repeat(1025);
        observe(&sink, &oversize); // seq 1: rejected before serialization.
        observe(&sink, "after-oversize"); // seq 2: accepted in the same batch.
        sink.flush();
        let values = diagnostic_values(&recorder);
        assert_eq!(values.len(), 2);
        assert_eq!(values[0]["data"]["kind"], "loss");
        assert_eq!(values[0]["data"]["sequence"], 1);
        assert_eq!(values[0]["data"]["droppedCount"], 1);
        assert_eq!(values[1]["data"]["kind"], "response");
        assert_eq!(values[1]["data"]["sequence"], 2);
        assert_eq!(values[1]["data"]["droppedCount"], 1);

        observe(&sink, &oversize); // seq 3: loss must be visible without an accepted response.
        sink.flush();
        let values = diagnostic_values(&recorder);
        assert_eq!(values.len(), 3);
        assert_eq!(values[2]["data"]["kind"], "loss");
        assert_eq!(values[2]["data"]["sequence"], 3);
        assert_eq!(values[2]["data"]["droppedCount"], 2);
        observe(&sink, "after-loss-flush"); // seq 4: accepted in the next batch.
        sink.flush();
        observe(&sink, "before-middle-loss"); // seq 5.
        observe(&sink, &oversize); // seq 6.
        observe(&sink, "after-middle-loss"); // seq 7.
        sink.flush();
        let values = diagnostic_values(&recorder);
        assert_eq!(values.len(), 7);
        assert_eq!(
            values
                .iter()
                .map(|v| v["data"]["sequence"].as_u64().unwrap())
                .collect::<Vec<_>>(),
            vec![1, 2, 3, 4, 5, 6, 7]
        );
        assert_eq!(values[3]["data"]["droppedCount"], 2);
        assert_eq!(values[5]["data"]["kind"], "loss");
        assert_eq!(values[5]["data"]["droppedCount"], 3);
        assert_eq!(values[6]["data"]["droppedCount"], 3);
        assert_eq!(recorder.batches.lock().len(), 4);
        observe(&sink, &oversize); // seq 8: first loss in this aggregate.
        observe(&sink, "between-losses"); // seq 9, cumulative drops 4.
        observe(&sink, &oversize); // seq 10.
        observe(&sink, &oversize); // seq 11: aggregate loss sequence, cumulative drops 6.
        observe(&sink, "after-multiple-losses"); // seq 12.
        sink.flush();
        let values = diagnostic_values(&recorder);
        assert_eq!(values.len(), 10);
        assert_eq!(
            values
                .iter()
                .map(|v| v["data"]["sequence"].as_u64().unwrap())
                .collect::<Vec<_>>(),
            vec![1, 2, 3, 4, 5, 6, 7, 9, 11, 12]
        );
        assert_eq!(values[7]["data"]["kind"], "response");
        assert_eq!(values[7]["data"]["droppedCount"], 4);
        assert_eq!(values[8]["data"]["kind"], "loss");
        assert_eq!(values[8]["data"]["droppedCount"], 6);
        assert_eq!(values[9]["data"]["kind"], "response");
        assert_eq!(values[9]["data"]["droppedCount"], 6);
        sink.flush();
        assert_eq!(recorder.batches.lock().len(), 5);
    }

    #[test]
    fn upload_diagnostic_missing_callback_retains_unreported_facts_and_failure() {
        let metrics = Arc::new(RecordingMetrics::default());
        let sink = FfiEventSink::new(CppHostVtable {
            on_domain_events_batch: None,
            now_ms: None,
            ctx: std::ptr::null_mut(),
        })
        .with_metric_sink(metrics.clone());
        sink.emit(DomainEventBytes(Bytes::from_static(b"original-business")));
        observe(&sink, "actual-key");
        observe(&sink, &"k".repeat(1025));
        sink.flush();
        sink.flush();
        let diagnostics = sink.upload_diagnostics.lock();
        assert_eq!(diagnostics.queue.len(), 1);
        assert_eq!(diagnostics.dropped_count, 1);
        assert_eq!(diagnostics.reported_dropped_count, 0);
        assert!(
            diagnostics.has_pending(),
            "no callback is not diagnostic delivery"
        );
        assert!(
            sink.buffer.lock().is_empty(),
            "existing missing-callback business policy stays intact"
        );
        let events = metrics.0.lock();
        assert_eq!(
            events
                .iter()
                .filter(|e| e.id == MetricId::FfiCallbackErrorsTotal)
                .count(),
            2
        );
        assert!(events
            .iter()
            .any(|e| e.id == MetricId::FfiEventsDroppedTotal && e.value == 1.0));
    }

    #[test]
    fn upload_diagnostic_concurrent_producers_flush_and_join_preserve_every_attempt_fact() {
        use std::collections::HashSet;
        use std::sync::Barrier;
        const PRODUCERS: usize = 4;
        const ATTEMPTS: usize = 500;
        let recorder = Arc::new(BatchRecorder::default());
        let sink = Arc::new(recording_sink(&recorder));
        let barrier = Arc::new(Barrier::new(PRODUCERS + 1));
        let done = Arc::new(AtomicUsize::new(0));
        let mut workers = Vec::new();
        for _ in 0..PRODUCERS {
            let (sink, barrier, done) = (sink.clone(), barrier.clone(), done.clone());
            workers.push(std::thread::spawn(move || {
                barrier.wait();
                for _ in 0..ATTEMPTS {
                    observe(&sink, "parallel-key");
                }
                done.fetch_add(1, Ordering::Release);
            }));
        }
        barrier.wait();
        while done.load(Ordering::Acquire) != PRODUCERS {
            sink.flush();
            assert!(sink.upload_diagnostics.lock().queue.len() <= 128);
            std::thread::yield_now();
        }
        for worker in workers {
            worker.join().unwrap();
        }
        sink.flush();
        assert!(!sink.upload_diagnostics.lock().has_pending());
        let values = diagnostic_values(&recorder);
        assert!(values.windows(2).all(|pair| {
            pair[0]["data"]["sequence"].as_u64().unwrap()
                < pair[1]["data"]["sequence"].as_u64().unwrap()
        }));
        let accepted: Vec<_> = values
            .iter()
            .filter(|v| v["data"]["kind"] == "response")
            .collect();
        let max_drops = values
            .iter()
            .map(|v| v["data"]["droppedCount"].as_u64().unwrap())
            .max()
            .unwrap_or(0);
        assert_eq!(
            accepted.len() as u64 + max_drops,
            (PRODUCERS * ATTEMPTS) as u64
        );
        let sequences: HashSet<_> = accepted
            .iter()
            .map(|v| v["data"]["sequence"].as_u64().unwrap())
            .collect();
        assert_eq!(sequences.len(), accepted.len());
        assert!(recorder
            .callback_threads
            .lock()
            .iter()
            .all(|id| *id == std::thread::current().id()));
        for batch in recorder.batches.lock().iter() {
            assert!(crate::batch::decode_batch(batch)
                .unwrap()
                .iter()
                .all(|event| event.len() <= 1024));
        }
        let callbacks = recorder.batches.lock().len();
        sink.flush();
        assert_eq!(recorder.batches.lock().len(), callbacks);
        drop(sink); // All producers have joined before callback context may be released.
        assert_eq!(recorder.batches.lock().len(), callbacks);
    }

    struct NotifyingSink {
        sink: Arc<FfiEventSink>,
        flushed: tokio::sync::mpsc::UnboundedSender<()>,
    }
    impl EventSink for NotifyingSink {
        fn emit(&self, event: DomainEventBytes) {
            self.sink.emit(event);
        }
    }
    impl BatchSink for NotifyingSink {
        fn flush(&self) {
            self.sink.flush();
            self.flushed.send(()).unwrap();
        }
    }
    #[derive(Clone)]
    struct FixedClock;
    impl helix_core::ports::Clock for FixedClock {
        fn now_ms(&self) -> u64 {
            1
        }
    }
    struct RejectDisconnected;
    impl helix_core::module_host::Module for RejectDisconnected {
        fn name(&self) -> &'static str {
            "reject-disconnected-test"
        }
        fn accepts(&self, tick: &helix_core::Tick) -> bool {
            matches!(tick, helix_core::Tick::Disconnected(_))
        }
        fn handle(
            &mut self,
            _: &helix_core::Tick,
            _: u64,
            _: &mut helix_core::effect::EffectSink,
        ) -> Result<(), helix_core::CoreError> {
            Err(helix_core::CoreError::NoHandler("test rejection"))
        }
    }
    struct UnusedFrameSender;
    #[async_trait::async_trait]
    impl helix_core::ports::FrameSender for UnusedFrameSender {
        async fn send(&self, _: Bytes) -> Result<(), helix_core::PortError> {
            unreachable!()
        }
    }

    #[tokio::test]
    async fn upload_diagnostic_late_response_delivers_on_empty_effect_step_once() {
        use helix_core::effect::TransportId;
        use helix_core::{ExecutionShell, Tick};
        use helix_driver_host::{
            run_engine_loop, EngineDeps, HostNetworkConfig, HostStorage, SharedFileUploader,
            SharedHttpClient, TraceHooks,
        };
        use std::collections::HashMap;
        use std::time::Duration;
        use tokio::sync::{mpsc, oneshot};
        let recorder = Arc::new(BatchRecorder::default());
        let sink = Arc::new(recording_sink(&recorder));
        let (flushed_tx, mut flushed_rx) = mpsc::unbounded_channel();
        let (tick_tx, tick_rx) = mpsc::channel(4);
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let (_transport_tx, transport_rx) = mpsc::unbounded_channel::<
            helix_driver_host::TransportRegistration<UnusedFrameSender>,
        >();
        let deps = EngineDeps {
            storage: Arc::new(HostStorage::open_sqlite_url(":memory:").await.unwrap()),
            http: Arc::new(
                SharedHttpClient::new(HostNetworkConfig::new(
                    "http://127.0.0.1:9",
                    "ws://127.0.0.1:9",
                ))
                .unwrap(),
            ),
            uploader: Arc::new(SharedFileUploader::default().with_observer(sink.clone())),
            event_sink: Arc::new(NotifyingSink {
                sink: sink.clone(),
                flushed: flushed_tx,
            }),
            clock: FixedClock,
            trace: TraceHooks::noop(),
            metrics: Arc::new(NoopMetricSink),
            max_http_inflight: 1,
            transport_lifecycle_tx: None,
            transport_trace_tx: None,
        };
        let mut shell = ExecutionShell::new();
        shell.register(RejectDisconnected);
        let engine = tokio::spawn(run_engine_loop(
            shell,
            tick_rx,
            tick_tx.clone(),
            deps,
            shutdown_rx,
            HashMap::new(),
            transport_rx,
        ));
        tokio::time::timeout(Duration::from_secs(2), flushed_rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(
            recorder.batches.lock().is_empty(),
            "empty start sends no callback"
        );
        observe(&sink, "late-key");
        assert!(
            recorder.batches.lock().is_empty(),
            "async observation cannot call C++ directly"
        );
        tick_tx
            .send(Tick::Connected(TransportId::from_raw(0)))
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(2), flushed_rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(recorder.batches.lock().len(), 1);
        assert_eq!(
            diagnostic_values(&recorder)[0]["data"]["objectKey"],
            "late-key"
        );
        tick_tx
            .send(Tick::Connected(TransportId::from_raw(0)))
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(2), flushed_rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            recorder.batches.lock().len(),
            1,
            "second empty step has no empty callback"
        );
        sink.observe(UploadResponseObservation {
            method: "PUT",
            http_status: None,
            object_key: "no-response-key",
            file_size: 7,
            duration_us: 150000,
        });
        tick_tx
            .send(Tick::Disconnected(TransportId::from_raw(0)))
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(2), flushed_rx.recv())
            .await
            .unwrap()
            .unwrap();
        let values = diagnostic_values(&recorder);
        assert_eq!(
            recorder.batches.lock().len(),
            2,
            "rejected step also has one driver batch boundary"
        );
        assert!(values[1]["data"]["httpStatus"].is_null());
        assert_eq!(values[1]["data"]["kind"], "response");
        assert_eq!(values[1]["data"]["objectKey"], "no-response-key");
        shutdown_tx.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(2), engine)
            .await
            .unwrap()
            .unwrap();
        let callbacks = recorder.batches.lock().len();
        drop(sink);
        assert_eq!(
            recorder.batches.lock().len(),
            callbacks,
            "joined engine and dropped sink have completed all callbacks"
        );
    }
}
