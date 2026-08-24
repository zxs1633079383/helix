use super::*;
use crate::metrics::{NoopMetricSink, RecordOutcome};
use bytes::Bytes;
use helix_core::effect::DomainEventBytes;
use helix_core::effect::{Correlation, TimerId};
use helix_core::tick::ReplyBytes;
use std::sync::Mutex;

#[derive(Default)]
struct RecordingMetricSink(Mutex<Vec<MetricEvent>>);

impl AsyncMetricSink for RecordingMetricSink {
    /// 收集聚焦测试产生的指标事件，避免接入真实 exporter。
    fn try_record(&self, event: MetricEvent) -> RecordOutcome {
        self.0.lock().unwrap().push(event);
        RecordOutcome::Accepted
    }
}

/// Engine 指标记录器必须保持小对象，避免占满 Tokio worker 的默认任务栈。
#[test]
fn engine_metric_recorder_remains_small_on_tokio_task_stack() {
    assert!(
        std::mem::size_of::<EngineMetricRecorder>() <= 512,
        "EngineMetricRecorder 不得把固定 correlation 表内联进 async task 栈"
    );
}

#[test]
fn pending_port_correlation_is_strictly_bounded() {
    let mut recorder = EngineMetricRecorder::new(Arc::new(NoopMetricSink));
    for corr in 0..5000 {
        recorder.remember_port_start(corr, "persist");
    }
    assert_eq!(
        recorder.port_slots.iter().flatten().count(),
        MAX_PENDING_PORTS
    );
}

#[test]
fn insert_reply_cycles_do_not_accumulate_stale_order_entries() {
    let mut recorder = EngineMetricRecorder::new(Arc::new(NoopMetricSink));
    for corr in 0..(MAX_PENDING_PORTS as u64 * 3) {
        recorder.remember_port_start(corr, "persist");
        assert!(recorder.complete_port(corr).is_some());
    }
    assert_eq!(recorder.port_slots.iter().flatten().count(), 0);
}

#[test]
fn direct_slot_collision_evicts_old_correlation_without_allocation() {
    let mut recorder = EngineMetricRecorder::new(Arc::new(NoopMetricSink));
    recorder.remember_port_start(7, "persist");
    recorder.remember_port_start(7 + MAX_PENDING_PORTS as u64, "persist");
    assert!(recorder.complete_port(7).is_none());
    assert!(recorder
        .complete_port(7 + MAX_PENDING_PORTS as u64)
        .is_some());
}

/// collision 替换不能虚增 pending，完成新 correlation 后必须归零。
#[test]
fn pending_gauge_remains_exact_across_collision() {
    let metrics = Arc::new(RecordingMetricSink::default());
    let mut recorder = EngineMetricRecorder::new(metrics.clone());
    recorder.remember_port_start(7, "persist");
    recorder.remember_port_start(7 + MAX_PENDING_PORTS as u64, "persist");
    assert_eq!(recorder.pending_ports, 1);
    assert!(recorder
        .complete_port(7 + MAX_PENDING_PORTS as u64)
        .is_some());
    recorder.record_pending_ports();

    let events = metrics.0.lock().unwrap();
    assert!(events
        .iter()
        .any(|event| event.id == MetricId::PortCorrelationCollisionTotal));
    assert!(events
        .iter()
        .any(|event| event.id == MetricId::PortPending && event.value == 0.0));
}

/// PortReply 必须用槽内保存的 command 根起点闭合跨 Tick HTTP response 阶段。
#[test]
fn port_reply_closes_cross_tick_command_stage() {
    let metrics = Arc::new(RecordingMetricSink::default());
    let mut recorder = EngineMetricRecorder::new(metrics.clone());
    recorder.active_command_started = Some(Instant::now());
    recorder.remember_port_start(41, "http");
    recorder.active_command_started = None;

    recorder.on_tick(
        &Tick::PortReply {
            corr: Correlation::from_raw(41),
            outcome: PortOutcome::Ok(ReplyBytes::default()),
        },
        None,
    );

    let events = metrics.0.lock().unwrap();
    assert!(events
        .iter()
        .any(|event| event.id == MetricId::CommandToHttpResponseSeconds));
}

/// command_terminal_requires_local_command_lineage 防止远端同名 projection 被误算为本地 Command 成功。
#[test]
fn command_terminal_requires_local_command_lineage() {
    let metrics = Arc::new(RecordingMetricSink::default());
    let mut recorder = EngineMetricRecorder::new(metrics.clone());
    let event = DomainEventBytes(Bytes::from_static(
        br#"{"event":"im:post:received","data":{}}"#,
    ));

    recorder.on_effect_dispatch(&OwnedEffect::Emit {
        event: event.clone(),
    });
    recorder.active_command_started = Some(Instant::now());
    recorder.on_effect_dispatch(&OwnedEffect::Emit {
        event: event.clone(),
    });
    recorder.on_effect_dispatch(&OwnedEffect::Emit { event });

    let events = metrics.0.lock().unwrap();
    assert_eq!(
        events
            .iter()
            .filter(|event| event.id == MetricId::ImCommandTerminalTotal)
            .count(),
        1,
    );
}

#[test]
fn stamped_tick_records_real_queue_wait_sample() {
    let metrics = Arc::new(RecordingMetricSink::default());
    let mut recorder = EngineMetricRecorder::new(metrics.clone());
    recorder.on_tick(
        &Tick::Timer(TimerId::from_raw(1)),
        Some(Duration::from_millis(7)),
    );
    let events = metrics.0.lock().unwrap();
    let sample = events
        .iter()
        .find(|event| event.id == MetricId::TickQueueWaitSeconds)
        .expect("stamped tick 必须即时记录 queue wait");
    assert!(sample.value >= 0.007);
}

#[test]
fn core_step_error_has_error_operation_denominator() {
    let metrics = Arc::new(RecordingMetricSink::default());
    let recorder = EngineMetricRecorder::new(metrics.clone());
    let context = TickMetricContext {
        tick_kind: "command",
        ingress_started: None,
        command_started: None,
    };
    recorder.on_step_error(&context);
    let events = metrics.0.lock().unwrap();
    let operation = events
        .iter()
        .find(|event| event.id == MetricId::OperationsTotal)
        .expect("step error 必须进入 operation 分母");
    assert!(operation
        .labels
        .iter()
        .any(|label| label.key == LabelKey::Status && label.value == "error"));
}
