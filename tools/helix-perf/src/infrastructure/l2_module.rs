use std::sync::atomic::Ordering;
use std::sync::Arc;

use bytes::Bytes;
use helix_core::effect::{DomainEventBytes, HttpRequest, SqlValue, StorageOp, TimerId, UpsertSpec};
use helix_core::{CoreError, Effect, EffectSink, Module, Tick};

use super::l2_oracle::L2Oracle;

pub struct PerfModule {
    oracle: Arc<L2Oracle>,
}

impl PerfModule {
    pub fn new(oracle: Arc<L2Oracle>) -> Self {
        Self { oracle }
    }
}

impl Module for PerfModule {
    fn name(&self) -> &'static str {
        "helix-perf-l2"
    }

    fn accepts(&self, tick: &Tick) -> bool {
        matches!(tick, Tick::Inbound(_))
            || matches!(tick, Tick::Command(command) if command.name == "perf_metric_probe")
    }

    fn handle(&mut self, tick: &Tick, _now_ms: u64, out: &mut EffectSink) -> Result<(), CoreError> {
        match tick {
            Tick::Command(command) if command.name == "perf_metric_probe" => {
                // 走真实 Command → core → dispatch 完成边界，同时不污染 L2 业务 oracle。
                out.push(Effect::CancelTimer {
                    id: TimerId::from_raw(u64::MAX),
                });
            }
            Tick::Inbound(frame) => {
                let value: serde_json::Value = serde_json::from_slice(frame.as_bytes())
                    .map_err(|_| CoreError::NoHandler("invalid perf frame"))?;
                let channel = value
                    .get("channel")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(u64::MAX) as usize;
                let seq = value.get("seq").and_then(|v| v.as_u64()).unwrap_or(0);
                let operation = value.get("operation").and_then(|v| v.as_u64()).unwrap_or(0);
                self.oracle.observe_dequeue(operation, channel, seq);
                let persist_corr = helix_core::Correlation::from_raw(
                    operation.saturating_mul(2).saturating_add(1),
                );
                let http_corr = helix_core::Correlation::from_raw(
                    operation.saturating_mul(2).saturating_add(2),
                );
                self.oracle
                    .mark_port(persist_corr.raw(), operation, "storage_port_roundtrip");
                self.oracle
                    .mark_port(http_corr.raw(), operation, "http_port_roundtrip");
                self.oracle.mark_event(operation);
                out.push(Effect::Persist {
                    corr: persist_corr,
                    ops: vec![StorageOp::BatchUpsert(UpsertSpec::new(
                        "perf_events",
                        vec![vec![
                            (
                                "event_key".into(),
                                SqlValue::Text(format!("{channel}:{seq}")),
                            ),
                            ("channel_no".into(), SqlValue::Integer(channel as i64)),
                            ("seq".into(), SqlValue::Integer(seq as i64)),
                            ("body".into(), SqlValue::Blob(frame.as_bytes().to_vec())),
                        ]],
                        Some("event_key"),
                    ))],
                });
                out.push(Effect::Http {
                    corr: http_corr,
                    req: HttpRequest {
                        method: "POST".into(),
                        url: "/perf".into(),
                        headers: Vec::new(),
                        body: Some(Bytes::copy_from_slice(frame.as_bytes())),
                    },
                });
                out.push(Effect::Emit {
                    event: DomainEventBytes(Bytes::copy_from_slice(frame.as_bytes())),
                });
            }
            Tick::PortReply { corr, outcome } => {
                self.oracle.observe_port_reply(corr.raw());
                match outcome {
                    helix_core::tick::PortOutcome::Ok(_) => {
                        self.oracle.port_replies_ok.fetch_add(1, Ordering::Relaxed);
                    }
                    helix_core::tick::PortOutcome::Err(_) => {
                        self.oracle.port_replies_err.fetch_add(1, Ordering::Relaxed);
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }
}
