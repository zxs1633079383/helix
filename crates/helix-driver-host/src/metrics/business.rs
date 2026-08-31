use helix_core::effect::DomainEventBytes;

use super::{AsyncMetricSink, LabelKey, MetricEvent, MetricId, MetricLabels};

const MESSAGE_V3_EVENT_PREFIX: &[u8] = b"{\"event\":\"";

/// ClientTerminalObservation 是边缘 UI 只读事实；所有字段都必须来自固定低基数枚举。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClientTerminalObservation {
    pub terminal: &'static str,
    pub path: &'static str,
    pub role: &'static str,
    pub result: &'static str,
    pub platform: &'static str,
    pub recipient_scope: &'static str,
}

/// record_message_client_terminal 将客户端终点旁路写入有界 sink，不参与业务 Tick 队列。
pub fn record_message_client_terminal(
    metrics: &dyn AsyncMetricSink,
    observation: ClientTerminalObservation,
) {
    let labels = MetricLabels::one(LabelKey::Terminal, observation.terminal)
        .with(LabelKey::Path, observation.path)
        .with(LabelKey::Role, observation.role)
        .with(LabelKey::Result, observation.result)
        .with(LabelKey::Platform, observation.platform)
        .with(LabelKey::RecipientScope, observation.recipient_scope);
    let _ = metrics.try_record(MetricEvent::counter(
        MetricId::ImMessageClientTerminalTotal,
        1.0,
        labels,
    ));
}

/// record_message_e2e_view_updated 记录同一客户端单调时钟产生的真实端到端样本。
pub fn record_message_e2e_view_updated(
    metrics: &dyn AsyncMetricSink,
    seconds: f64,
    path: &'static str,
    result: &'static str,
    platform: &'static str,
) {
    let labels = MetricLabels::one(LabelKey::Path, path)
        .with(LabelKey::Result, result)
        .with(LabelKey::Platform, platform);
    let _ = metrics.try_record(MetricEvent::histogram(
        MetricId::ImMessageE2eViewUpdatedDurationSeconds,
        seconds,
        labels,
    ));
}

/// record_seq_observation 记录已由本地同步状态机判定的固定 Seq 结果。
pub fn record_seq_observation(
    metrics: &dyn AsyncMetricSink,
    outcome: &'static str,
    path: &'static str,
    operation: &'static str,
) {
    let labels = MetricLabels::one(LabelKey::State, outcome)
        .with(LabelKey::Path, path)
        .with(LabelKey::Operation, operation);
    let _ = metrics.try_record(MetricEvent::counter(
        MetricId::ImSeqObservationTotal,
        1.0,
        labels,
    ));
}

/// record_gap_duration 仅在 gap lifecycle 形成稳定终态时记录持续时间。
pub fn record_gap_duration(
    metrics: &dyn AsyncMetricSink,
    seconds: f64,
    terminal: &'static str,
    path: &'static str,
    result: &'static str,
) {
    let labels = MetricLabels::one(LabelKey::Terminal, terminal)
        .with(LabelKey::Path, path)
        .with(LabelKey::Result, result);
    let _ = metrics.try_record(MetricEvent::histogram(
        MetricId::ImGapDurationSeconds,
        seconds,
        labels,
    ));
}

/// record_tick_stage_duration 记录已有 Tick 生命周期边界计算出的阶段样本。
pub fn record_tick_stage_duration(
    metrics: &dyn AsyncMetricSink,
    seconds: f64,
    stage: &'static str,
    result: &'static str,
) {
    let labels = MetricLabels::one(LabelKey::Stage, stage).with(LabelKey::Result, result);
    let _ = metrics.try_record(MetricEvent::histogram(
        MetricId::ImTickStageDurationSeconds,
        seconds,
        labels,
    ));
}

/// record_im_business_event 在 host Emit 边界记录投影、同步与恢复终态，不把远端推送冒充本地 Command。
pub fn record_im_business_event(metrics: &dyn AsyncMetricSink, event: &DomainEventBytes) {
    if !metrics.is_enabled() {
        return;
    }
    let Some(event_name) = message_v3_event_name(event.0.as_ref()) else {
        return;
    };

    match event_name {
        b"im:post:received" => {
            record_terminal(
                metrics,
                MetricId::ImProjectionTerminalTotal,
                "send_message",
                "success",
                "none",
            );
            record_terminal(
                metrics,
                MetricId::ImMessageCorrectnessTotal,
                "message_projection",
                "success",
                "none",
            );
        }
        b"im:post:send-failed" => {}
        b"im:post:client-ack-succeeded" => record_terminal(
            metrics,
            MetricId::ImClientAckTerminalTotal,
            "client_ack",
            "success",
            "none",
        ),
        b"im:post:client-ack-failed" => record_terminal(
            metrics,
            MetricId::ImClientAckTerminalTotal,
            "client_ack",
            "error",
            "ack_failed",
        ),
        b"im:post:increment-failed" => {
            record_terminal(
                metrics,
                MetricId::ImProjectionTerminalTotal,
                "channel_increment",
                "error",
                "increment_failed",
            );
            record_terminal(
                metrics,
                MetricId::ImSyncAnomalyTotal,
                "channel_increment",
                "error",
                "increment_failed",
            );
        }
        b"im:channel-sync-complete" => {
            record_terminal(
                metrics,
                MetricId::ImProjectionTerminalTotal,
                "channel_sync",
                "success",
                "none",
            );
            record_terminal(
                metrics,
                MetricId::ImSyncSessionTotal,
                "channel_sync",
                "success",
                "none",
            );
        }
        b"im:sync:loaded" => record_terminal(
            metrics,
            MetricId::ImSyncSessionTotal,
            "startup_sync",
            "success",
            "none",
        ),
        b"im:sync:recovered" => {
            record_terminal(
                metrics,
                MetricId::ImSyncSessionTotal,
                "offline_recovery",
                "success",
                "none",
            );
            record_terminal(
                metrics,
                MetricId::ImRecoverySessionTotal,
                "offline_recovery",
                "success",
                "none",
            );
        }
        b"im:sync:gap-repaired" => {
            record_terminal(
                metrics,
                MetricId::ImRecoverySessionTotal,
                "gap_repair",
                "success",
                "none",
            );
            record_terminal(
                metrics,
                MetricId::ImMessageCorrectnessTotal,
                "gap_repair",
                "success",
                "gap_repaired",
            );
        }
        b"im:sync:channel-hydrated" => record_terminal(
            metrics,
            MetricId::ImRecoverySessionTotal,
            "channel_hydration",
            "success",
            "none",
        ),
        b"im:sync:too_long" => {
            record_terminal(
                metrics,
                MetricId::ImSyncSessionTotal,
                "channel_sync",
                "error",
                "too_long",
            );
            record_terminal(
                metrics,
                MetricId::ImSyncAnomalyTotal,
                "channel_sync",
                "error",
                "too_long",
            );
            record_terminal(
                metrics,
                MetricId::ImMessageCorrectnessTotal,
                "channel_sync",
                "error",
                "gap",
            );
        }
        b"im:post:updated" | b"im:post:updates" | b"im:post:batch-updated" => {
            record_projection(metrics, "update_message")
        }
        b"im:post:revoke" => record_projection(metrics, "revoke_message"),
        b"im:post:deleted" => record_projection(metrics, "delete_message"),
        b"im:post:read" | b"im:post:readers" | b"im:channel:read_echo" => {
            record_projection(metrics, "mark_read")
        }
        b"im:channel:created" => record_projection(metrics, "create_channel"),
        b"im:channel:closed" => record_projection(metrics, "close_channel"),
        b"im:channel:schedule-created" => record_projection(metrics, "create_schedule"),
        b"im:channel:schedule-canceled" => record_projection(metrics, "cancel_schedule"),
        b"im:channel:member-updated" | b"im:channel:member-nickname" => {
            record_projection(metrics, "update_channel_member")
        }
        b"im:channel:settings-updated" => record_projection(metrics, "update_channel_settings"),
        b"im:todo:updated" => record_projection(metrics, "update_todo"),
        b"im:post_chain:publish"
        | b"im:post_chain:upsert"
        | b"im:post_chain:close"
        | b"im:post_chain:retract"
        | b"im:post_chain:read_cursor" => record_projection(metrics, "post_chain"),
        b"im:post_chain:append_rejected" => {}
        b"im:read:result" => record_unread_reconcile(metrics, event),
        _ => {}
    }
}

/// record_unread_reconcile 只接受 hydration 终态中同一 authority snapshot 的双侧绝对值结果。
fn record_unread_reconcile(metrics: &dyn AsyncMetricSink, event: &DomainEventBytes) {
    let Ok(payload) = serde_json::from_slice::<serde_json::Value>(event.0.as_ref()) else {
        return;
    };
    let Some(status) = payload
        .pointer("/data/body/unreadReconcile/status")
        .and_then(serde_json::Value::as_str)
    else {
        tracing::debug!("unread reconcile terminal absent from read result");
        return;
    };
    tracing::info!(status, "unread reconcile terminal recorded");
    match status {
        "match" => record_terminal(
            metrics,
            MetricId::ImUnreadReconcileTotal,
            "unread_reconcile",
            "match",
            "none",
        ),
        "mismatch" => record_terminal(
            metrics,
            MetricId::ImUnreadReconcileTotal,
            "unread_reconcile",
            "mismatch",
            "unread_mismatch",
        ),
        _ => {}
    }
}

/// record_im_command_terminal_event 仅在 Engine 已证明本地 Command lineage 时记录首个稳定终态。
pub(crate) fn record_im_command_terminal_event(
    metrics: &dyn AsyncMetricSink,
    event: &DomainEventBytes,
) -> bool {
    let Some(event_name) = message_v3_event_name(event.0.as_ref()) else {
        return false;
    };
    let Some((operation, status, error_kind)) = command_terminal(event_name) else {
        return false;
    };
    record_terminal(
        metrics,
        MetricId::ImCommandTerminalTotal,
        operation,
        status,
        error_kind,
    );
    true
}

/// command_terminal 把固定 MessageV3 终态事件映射成低基数 Command 结果。
fn command_terminal(event_name: &[u8]) -> Option<(&'static str, &'static str, &'static str)> {
    match event_name {
        b"im:post:received" => Some(("send_message", "success", "none")),
        b"im:post:send-failed" => Some(("send_message", "error", "send_failed")),
        b"im:post:updated" | b"im:post:updates" | b"im:post:batch-updated" => {
            Some(("update_message", "success", "none"))
        }
        b"im:post:revoke" => Some(("revoke_message", "success", "none")),
        b"im:post:deleted" => Some(("delete_message", "success", "none")),
        b"im:post:read" | b"im:post:readers" | b"im:channel:read_echo" => {
            Some(("mark_read", "success", "none"))
        }
        b"im:channel:created" => Some(("create_channel", "success", "none")),
        b"im:channel:closed" => Some(("close_channel", "success", "none")),
        b"im:channel:schedule-created" => Some(("create_schedule", "success", "none")),
        b"im:channel:schedule-canceled" => Some(("cancel_schedule", "success", "none")),
        b"im:channel:member-updated" | b"im:channel:member-nickname" => {
            Some(("update_channel_member", "success", "none"))
        }
        b"im:channel:settings-updated" => Some(("update_channel_settings", "success", "none")),
        b"im:todo:updated" => Some(("update_todo", "success", "none")),
        b"im:post_chain:publish"
        | b"im:post_chain:upsert"
        | b"im:post_chain:close"
        | b"im:post_chain:retract"
        | b"im:post_chain:read_cursor" => Some(("post_chain", "success", "none")),
        b"im:post_chain:append_rejected" => Some(("post_chain", "error", "append_rejected")),
        _ => None,
    }
}

/// message_v3_event_name 只读取 canonical envelope 的有界 event 前缀，拒绝非标准或截断输入。
fn message_v3_event_name(bytes: &[u8]) -> Option<&[u8]> {
    let rest = bytes.strip_prefix(MESSAGE_V3_EVENT_PREFIX)?;
    let end = rest.iter().position(|byte| *byte == b'"')?;
    (end <= 96).then_some(&rest[..end])
}

/// record_projection 为 durable projection 记录唯一投影终态，Command 终态由 Engine lineage 单独闭合。
fn record_projection(metrics: &dyn AsyncMetricSink, operation: &'static str) {
    record_terminal(
        metrics,
        MetricId::ImProjectionTerminalTotal,
        operation,
        "success",
        "none",
    );
}

/// record_terminal 统一写入 operation/status/error_kind 三个静态低基数标签。
fn record_terminal(
    metrics: &dyn AsyncMetricSink,
    id: MetricId,
    operation: &'static str,
    status: &'static str,
    error_kind: &'static str,
) {
    let labels = MetricLabels::one(LabelKey::Operation, operation)
        .with(LabelKey::Status, status)
        .with(LabelKey::ErrorKind, error_kind);
    let _ = metrics.try_record(MetricEvent::counter(id, 1.0, labels));
}
