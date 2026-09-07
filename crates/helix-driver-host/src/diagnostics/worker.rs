//! 网络和JSON只在单后台worker执行，重试保留原始timestamp/body/event_id。
use super::{fields::Fields, DiagnosticConfig, DiagnosticSnapshot, Stats};
use crate::{AsyncMetricSink, LabelKey, MetricEvent, MetricId, MetricLabels};
use std::sync::{atomic::Ordering, Arc};
use tokio::sync::{mpsc, oneshot};

/// 顺序批量导出，关闭后排空已接受记录；网络失败有限重试后显式计丢弃。
pub(super) async fn run(
    config: DiagnosticConfig,
    mut rx: mpsc::Receiver<Fields>,
    mut stop: oneshot::Receiver<()>,
    stats: Arc<Stats>,
    metrics: Arc<dyn AsyncMetricSink>,
) {
    let client = match reqwest::Client::builder().timeout(config.timeout).build() {
        Ok(client) => client,
        Err(_) => {
            stats.errors.fetch_add(1, Ordering::Relaxed);
            return;
        }
    };
    let boot = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let mut sequence = 0u64;
    let mut closing = false;
    let mut previous = DiagnosticSnapshot::default();
    loop {
        let first = if closing {
            rx.recv().await
        } else {
            tokio::select! {biased; _=&mut stop=>{closing=true;rx.close();rx.recv().await},value=rx.recv()=>value,_=tokio::time::sleep(config.export_interval)=>{record_stats(&stats,&metrics,&mut previous);continue;}}
        };
        let Some(first) = first else {
            break;
        };
        let mut records = Vec::with_capacity(config.batch_max);
        records.push(first);
        if !closing {
            tokio::select! {_=&mut stop=>{closing=true;rx.close();},_=tokio::time::sleep(config.export_interval)=>{}}
        }
        while records.len() < config.batch_max {
            match rx.try_recv() {
                Ok(record) => records.push(record),
                Err(_) => break,
            }
        }
        stats.depth.fetch_sub(records.len(), Ordering::Relaxed);
        let count = records.len();
        let mut values = Vec::with_capacity(count);
        for record in records {
            sequence += 1;
            let timestamp = record.observed_ns;
            let mut body = record.json();
            body.insert("schema_version".into(), 1.into());
            body.insert(
                "event_id".into(),
                format!("{}-{boot}-{sequence}", std::process::id()).into(),
            );
            body.insert(
                "observed_at".into(),
                time::OffsetDateTime::from_unix_timestamp_nanos(timestamp as i128)
                    .unwrap_or(time::OffsetDateTime::UNIX_EPOCH)
                    .format(&time::format_description::well_known::Rfc3339)
                    .unwrap_or_default()
                    .into(),
            );
            body.insert(
                "producer_service".into(),
                config.producer_service.clone().into(),
            );
            body.insert(
                "source_revision".into(),
                config.source_revision.clone().into(),
            );
            body.insert(
                "dropped_records".into(),
                (stats.dropped.load(Ordering::Relaxed) + stats.invalid.load(Ordering::Relaxed))
                    .into(),
            );
            // 旁路无法证明全链路完整；业务率不能由日志条数推断。
            body.insert("evidence_complete".into(), false.into());
            values.push(serde_json::json!([
                timestamp.to_string(),
                serde_json::Value::Object(body).to_string()
            ]));
        }
        let body=serde_json::json!({"streams":[{"stream":{"service_name":"helix-im-diagnostics","deployment_environment":config.environment,"platform":config.platform},"values":values}]}).to_string();
        let mut success = false;
        for attempt in 0..=config.retry_max {
            match client
                .post(&config.loki_url)
                .header("Content-Type", "application/json")
                .body(body.clone())
                .send()
                .await
            {
                Ok(reply) if reply.status().is_success() => {
                    success = true;
                    break;
                }
                _ => {
                    stats.errors.fetch_add(1, Ordering::Relaxed);
                }
            }
            if attempt < config.retry_max {
                tokio::time::sleep(std::time::Duration::from_millis(100 * (1u64 << attempt))).await;
            }
        }
        if success {
            stats.exported.fetch_add(count as u64, Ordering::Relaxed);
        } else {
            stats.dropped.fetch_add(count as u64, Ordering::Relaxed);
        }
        record_stats(&stats, &metrics, &mut previous);
    }
    record_stats(&stats, &metrics, &mut previous);
}

/// 以delta旁路送入现有AsyncMetricSink，不给用户/群/序号建立时序标签。
fn record_stats(
    stats: &Stats,
    metrics: &Arc<dyn AsyncMetricSink>,
    previous: &mut DiagnosticSnapshot,
) {
    let current = stats.snapshot();
    for (stage, now, before) in [
        ("accepted", current.accepted, previous.accepted),
        ("exported", current.exported, previous.exported),
        ("dropped", current.dropped, previous.dropped),
        ("invalid", current.invalid, previous.invalid),
    ] {
        let _ = metrics.try_record(MetricEvent::counter(
            MetricId::DiagnosticRecordsTotal,
            now.saturating_sub(before) as f64,
            MetricLabels::one(LabelKey::Stage, stage),
        ));
    }
    let _ = metrics.try_record(MetricEvent::counter(
        MetricId::DiagnosticExportErrorsTotal,
        current.errors.saturating_sub(previous.errors) as f64,
        MetricLabels::EMPTY,
    ));
    let _ = metrics.try_record(MetricEvent::gauge(
        MetricId::DiagnosticQueueDepth,
        current.queue_depth as f64,
        MetricLabels::EMPTY,
    ));
    *previous = current;
}
