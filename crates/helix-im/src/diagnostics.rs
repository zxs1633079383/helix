//! 业务事实的只读旁路；不分配Effect、不更改业务状态、不读取时钟或执行I/O。
use crate::module::ImModule;

#[derive(Default)]
pub(crate) struct Session {
    pub login: String,
    pub device: String,
    pub now_ms: u64,
    next_gap: u64,
    gaps: std::collections::BTreeMap<crate::state::ChannelId, Gap>,
}
struct Gap {
    id: u64,
    from: u64,
    through: u64,
    started_ms: u64,
}
#[derive(Default)]
pub(crate) struct Observation<'a> {
    pub event: &'static str,
    pub domain: &'static str,
    pub stage: &'static str,
    pub path: &'static str,
    pub result: &'static str,
    pub reason: &'static str,
    pub channel: &'a str,
    pub seq: Option<u64>,
    pub from: Option<u64>,
    pub next: Option<u64>,
    pub cursor: Option<u64>,
    pub expected: Option<u64>,
    pub received: Option<u64>,
    pub declared_from: Option<u64>,
    pub declared_to: Option<u64>,
    pub count: usize,
    pub corr: Option<u64>,
    pub has_more: bool,
    pub gap: Option<u64>,
    pub elapsed: f64,
}
struct OptionalNumber(Option<u64>);
impl std::fmt::Display for OptionalNumber {
    /// 缺失值保持空字段，由后台省略；不伪填序号0。
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.0 {
            Some(v) => write!(f, "{v}"),
            None => Ok(()),
        }
    }
}
impl ImModule {
    /// 宿主在注册模块前注入诊断会话；不改变认证身份或业务协议。
    pub fn with_diagnostic_session(mut self, login: String, device: String) -> Self {
        self.diagnostics = Session {
            login,
            device,
            ..Default::default()
        };
        self
    }
    /// 仅从已认证config取身份；固定标量借用，无正文clone/JSON/额外存储扫描。
    pub(crate) fn diagnose(&self, record: Observation<'_>) {
        if self.diagnostics.login.is_empty() {
            return;
        }
        tracing::info!(target:"helix::diagnostics",
            event=record.event,tenant_id=self.config.company_id.as_str(),user_id=self.config.auth_user_id.as_str(),
            login_attempt_id=self.diagnostics.login.as_str(),device_session_id=self.diagnostics.device.as_str(),
            channel_id=record.channel,stage=record.stage,path=record.path,result=record.result,reason=record.reason,
            seq_domain=if record.domain.is_empty() {"channel_cursor"} else {record.domain},event_seq=%OptionalNumber(record.seq),from_seq=%OptionalNumber(record.from),
            next_seq=%OptionalNumber(record.next),local_contiguous_seq=%OptionalNumber(record.cursor),
            expected_seq=%OptionalNumber(record.expected),received_seq=%OptionalNumber(record.received),
            declared_from_seq=%OptionalNumber(record.declared_from),declared_to_seq=%OptionalNumber(record.declared_to),
            corr=%OptionalNumber(record.corr),count=record.count as u64,has_more=record.has_more,expected_known=false,evidence_complete=false,
            gap_id=%GapKey{session:self.diagnostics.login.as_str(),channel:record.channel,id:record.gap},elapsed_seconds=record.elapsed
        );
    }
    /// 只读取已存在的单群cursor，表示本次观察的已提交边界，不计算max(seq)。
    pub(crate) fn diagnose_checkpoint(
        &mut self,
        channel: crate::state::ChannelId,
        source: &'static str,
    ) {
        if let Some(cursor) = self
            .state
            .channels
            .get(&channel)
            .map(|state| state.cursor.value().0)
        {
            self.diagnose(Observation {
                event: "channel_checkpoint",
                stage: source,
                channel: channel.as_str(),
                cursor: Some(cursor),
                ..Default::default()
            });
            if self
                .diagnostics
                .gaps
                .get(&channel)
                .is_some_and(|gap| cursor >= gap.through)
            {
                if let Some(gap) = self.diagnostics.gaps.remove(&channel) {
                    self.diagnose(Observation {
                        event: "gap_terminal",
                        result: "cursor_caught_up",
                        reason: "requires_persistence_reconciliation",
                        channel: channel.as_str(),
                        gap: Some(gap.id),
                        expected: Some(gap.from),
                        received: gap.through.checked_add(1),
                        cursor: Some(cursor),
                        elapsed: self.diagnostics.now_ms.saturating_sub(gap.started_ms) as f64
                            / 1000.0,
                        ..Default::default()
                    });
                }
            }
        }
    }
    /// Episode仅用于观测；有界映射不参与补偿或提交决策，满时显式报告截断。
    fn diagnose_gap(
        &mut self,
        channel: crate::state::ChannelId,
        from: u64,
        through: u64,
        reason: &'static str,
    ) {
        if from > through {
            return;
        }
        if self.diagnostics.gaps.len() >= 1024 && !self.diagnostics.gaps.contains_key(&channel) {
            self.diagnose(Observation {
                event: "diagnostic_truncated",
                reason: "capacity_limit",
                count: 1,
                ..Default::default()
            });
            return;
        }
        let existed = self.diagnostics.gaps.contains_key(&channel);
        if !existed {
            self.diagnostics.next_gap += 1;
            self.diagnostics.gaps.insert(
                channel,
                Gap {
                    id: self.diagnostics.next_gap,
                    from,
                    through,
                    started_ms: self.diagnostics.now_ms,
                },
            );
        }
        let Some(gap) = self.diagnostics.gaps.get_mut(&channel) else {
            return;
        };
        gap.through = gap.through.max(through);
        gap.from = gap.from.min(from);
        let (id, from, through) = (gap.id, gap.from, gap.through);
        self.diagnose(Observation {
            event: if existed { "gap_updated" } else { "gap_opened" },
            reason,
            channel: channel.as_str(),
            gap: Some(id),
            expected: Some(from),
            received: through.checked_add(1),
            ..Default::default()
        });
    }
    /// 已解析WS只记录标量，接收与业务接受/提交分开；pong区间原样保留不更改补偿。
    pub(crate) fn diagnose_inbound(&mut self, frame: &crate::ws::WsFrame) {
        if self.diagnostics.login.is_empty() {
            return;
        }
        let data = frame.data();
        let channel = data
            .and_then(|v| {
                v.get("channelId")
                    .or_else(|| v.get("channel_id"))
                    .or_else(|| {
                        v.get("post")
                            .and_then(|p| p.get("channel_id").or_else(|| p.get("channelId")))
                    })
            })
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");
        let seq = frame.event_seq().map(|value| value.0);
        let cursor = crate::state::ChannelId::from_str(channel)
            .and_then(|id| self.state.channels.get(&id))
            .map(|ch| ch.cursor.value().0);
        if !channel.is_empty() && seq.is_some() {
            self.diagnose(Observation {
                domain: if data
                    .is_some_and(|v| v.get("stream_seq").is_some() || v.get("streamSeq").is_some())
                {
                    "stream_seq"
                } else {
                    "legacy_event_seq"
                },
                event: "delivery_stage",
                stage: "ws_received",
                path: "live_ws",
                channel,
                seq,
                cursor,
                ..Default::default()
            });
        }
        if let (Some(seq), Some(cursor), Some(channel)) =
            (seq, cursor, crate::state::ChannelId::from_str(channel))
        {
            if seq > cursor.saturating_add(1) {
                self.diagnose_gap(
                    channel,
                    cursor.saturating_add(1),
                    seq - 1,
                    "pending_authority_check",
                );
            }
        }
        if let Some(gaps) = data
            .and_then(|v| v.get("gaps"))
            .and_then(serde_json::Value::as_array)
        {
            for gap in gaps.iter().take(128) {
                let Some(channel) = gap.get("channelId").and_then(serde_json::Value::as_str) else {
                    continue;
                };
                let number = |name| {
                    gap.get(name).and_then(|v| {
                        v.as_u64()
                            .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
                    })
                };
                let from = number("fromSeq");
                let to = number("maxSeq");
                self.diagnose(Observation {
                    event: "gap_observed",
                    reason: "server_reported_range",
                    path: "offline_recovery",
                    channel,
                    declared_from: from,
                    declared_to: to,
                    ..Default::default()
                });
            }
            if gaps.len() > 128 {
                self.diagnose(Observation {
                    event: "diagnostic_truncated",
                    reason: "capacity_limit",
                    count: gaps.len() - 128,
                    ..Default::default()
                });
            }
        }
    }
}

struct GapKey<'a> {
    session: &'a str,
    channel: &'a str,
    id: Option<u64>,
}
impl std::fmt::Display for GapKey<'_> {
    /// Session与群限定episode身份；不放入任何时序标签。
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.id {
            Some(id) => write!(f, "{}:{}:{id}", self.session, self.channel),
            None => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{ChannelId, Seq};
    /// 精确大序号缺口只维护有界观测状态；游标追上必须单独记录且不改业务cursor。
    #[test]
    fn gap_episode_preserves_exact_range_and_waits_for_committed_cursor() {
        let channel = ChannelId::from_str("chfixx00000000000000000001").unwrap();
        let mut module = ImModule::new(Default::default())
            .with_diagnostic_session("login".into(), "device".into());
        module.register_channel(channel, 9007199254740992);
        module.diagnostics.now_ms = 100;
        let frame=crate::ws::WsFrame::parse(br#"{"action":"post","data":{"channelId":"chfixx00000000000000000001","streamSeq":9007199254740995}}"#).unwrap();
        module.diagnose_inbound(&frame);
        let gap = module.diagnostics.gaps.get(&channel).unwrap();
        assert_eq!(
            (gap.from, gap.through),
            (9007199254740993, 9007199254740994)
        );
        let id = gap.id;
        module.diagnose_inbound(&frame);
        assert_eq!(module.diagnostics.gaps.get(&channel).unwrap().id, id);
        assert_eq!(module.cursor_for(channel), Some(9007199254740992));
        module.diagnose_checkpoint(channel, "test_readonly");
        assert_eq!(module.diagnostics.gaps.len(), 1);
        module
            .state
            .channels
            .get_mut(&channel)
            .unwrap()
            .cursor
            .try_advance(Seq(9007199254740995));
        module.diagnose_checkpoint(channel, "test_committed");
        assert!(module.diagnostics.gaps.is_empty());
    }
}
