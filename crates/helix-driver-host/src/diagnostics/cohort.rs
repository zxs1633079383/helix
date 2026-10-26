//! 后台有界窗口统计；只证明当前运行会话收到后30秒内的持久化确认。
use serde_json::{Map, Value};
use std::collections::BTreeMap;

const WINDOW_NS: u128 = 30_000_000_000;
const MAX_EVENTS: usize = 4096;
const MAX_GROUPS: usize = 1024;
const KEYS: [&str; 6] = [
    "tenant_id",
    "user_id",
    "device_session_id",
    "login_attempt_id",
    "channel_id",
    "seq_domain",
];
type Group = [String; 6];

struct Entry {
    received: u128,
    window: u128,
    terminal: Option<&'static str>,
    traceparent: String,
}

#[derive(Default)]
pub(super) struct Cohorts {
    entries: BTreeMap<(Group, u64), Entry>,
    retired: BTreeMap<Group, u64>,
    tainted: bool,
}

/// 只读取正文字符串；身份与序号禁止浮点转换。
fn text<'a>(body: &'a Map<String, Value>, key: &str) -> &'a str {
    body.get(key).and_then(Value::as_str).unwrap_or("")
}

impl Cohorts {
    /// 丢失不可恢复为完整统计；需要新的runtime才可重新建立完整观察窗口。
    pub fn mark_loss(&mut self, lost: bool) {
        self.tainted |= lost;
    }

    /// 每条入站后台O(log N)，活动表固定上限；返回明确的覆盖缺口或迟到诊断。
    pub fn observe(
        &mut self,
        body: &mut Map<String, Value>,
        now: u128,
    ) -> Option<Map<String, Value>> {
        if text(body, "event") != "delivery_stage" {
            return None;
        }
        let stage = text(body, "stage");
        if !matches!(stage, "ws_received" | "persist_terminal") {
            return None;
        }
        let group: Group = KEYS.map(|key| text(body, key).to_owned());
        let seq = text(body, "event_seq")
            .parse::<u64>()
            .ok()
            .filter(|s| *s > 0);
        if group.iter().any(String::is_empty)
            || !matches!(group[5].as_str(), "stream_seq" | "legacy_event_seq")
            || seq.is_none()
        {
            return Some(self.gap(body, "identity_or_sequence_missing"));
        }
        let seq = seq.unwrap_or_default();
        let key = (group.clone(), seq);
        if stage == "ws_received" {
            if text(body, "result") == "already_committed" {
                return None;
            }
            if self.entries.contains_key(&key) {
                return None;
            }
            if self.retired.get(&group).is_some_and(|high| seq <= *high) {
                return Some(self.gap(body, "retired_sequence_ambiguous"));
            }
            if self.entries.len() >= MAX_EVENTS
                || (!self.retired.contains_key(&group) && self.retired.len() >= MAX_GROUPS)
            {
                return Some(self.gap(body, "capacity_limit"));
            }
            self.retired.entry(group).or_insert(0);
            self.entries.insert(
                key,
                Entry {
                    received: now,
                    window: now / WINDOW_NS,
                    terminal: None,
                    traceparent: text(body, "traceparent").to_owned(),
                },
            );
            return None;
        }
        let result = match text(body, "result") {
            "success" => "committed",
            "failed" => "failed",
            _ => return Some(self.gap(body, "terminal_result_unknown")),
        };
        if let Some(entry) = self.entries.get_mut(&key) {
            if !entry.traceparent.is_empty() {
                body.entry("traceparent".to_string())
                    .or_insert_with(|| entry.traceparent.clone().into());
            }
            if now.saturating_sub(entry.received) <= WINDOW_NS {
                // 已收到真实提交确认后，重复的旧失败回执不能倒退持久化事实。
                if entry.terminal != Some("committed") {
                    entry.terminal = Some(result);
                }
                return None;
            }
        }
        // 回执迟到或来自未观察到的入站，不重开既有义务，也不增加成功分母。
        let mut late = body.clone();
        late.insert("event".into(), "late_terminal".into());
        late.insert("reason".into(), "outside_observed_deadline".into());
        Some(late)
    }

    /// 覆盖缺口明确归于当前scope；会话后续窗口全部失去完整性。
    fn gap(&mut self, body: &Map<String, Value>, reason: &'static str) -> Map<String, Value> {
        self.tainted = true;
        let mut output = Map::new();
        for key in KEYS {
            if let Some(value) = body.get(key) {
                output.insert(key.into(), value.clone());
            }
        }
        output.insert("event".into(), "diagnostic_coverage_gap".into());
        output.insert("reason".into(), reason.into());
        output.insert("unknown".into(), 1.into());
        output.insert("evidence_complete".into(), false.into());
        output
    }

    /// 每次后台flush扫描有界表O(N)；窗口最大60秒，关闭时pending明确unknown。
    pub fn close_ready(&mut self, now: u128, closing: bool) -> Vec<Map<String, Value>> {
        let keys: Vec<_> = self
            .entries
            .iter()
            .filter(|(_, entry)| closing || now >= (entry.window + 2) * WINDOW_NS)
            .map(|(key, _)| key.clone())
            .collect();
        let mut windows: BTreeMap<(Group, u128), [u64; 4]> = BTreeMap::new();
        for key in keys {
            let Some(entry) = self.entries.remove(&key) else {
                continue;
            };
            let counts = windows.entry((key.0.clone(), entry.window)).or_default();
            let slot = match entry.terminal {
                Some("committed") => 0,
                Some("failed") => 1,
                _ if closing => 3,
                _ => 2,
            };
            counts[slot] += 1;
            self.retired
                .entry(key.0)
                .and_modify(|high| *high = (*high).max(key.1));
        }
        windows
            .into_iter()
            .map(|((group, window), counts)| {
                let mut body = Map::new();
                let cohort_id = format!("{}:{window}", group[5]);
                for (key, value) in KEYS.into_iter().zip(group) {
                    body.insert(key.into(), value.into());
                }
                body.insert("event".into(), "delivery_cohort_closed".into());
                body.insert("cohort_id".into(), cohort_id.into());
                body.insert("evidence_scope".into(), "client_persistence".into());
                body.insert("path".into(), "live_ws".into());
                for (key, count) in ["committed", "failed", "timeout", "unknown"]
                    .into_iter()
                    .zip(counts)
                {
                    body.insert(key.into(), count.into());
                }
                body.insert("observed".into(), counts.iter().sum::<u64>().into());
                body.insert("matured".into(), (counts[0] + counts[1] + counts[2]).into());
                body.insert("pending".into(), 0.into());
                body.insert(
                    "evidence_complete".into(),
                    (!self.tainted && counts[3] == 0).into(),
                );
                body
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    /// 使用固定时钟和标量构造观察，不依赖网络或sleep。
    fn observation(stage: &str, seq: &str, result: &str) -> Map<String, Value> {
        serde_json::json!({"event":"delivery_stage","stage":stage,"result":result,"tenant_id":"t","user_id":"u","device_session_id":"d","login_attempt_id":"l","channel_id":"c","seq_domain":"stream_seq","event_seq":seq}).as_object().unwrap().clone()
    }
    /// 同窗重试失败后成功只记一次，精确大序号不经float。
    #[test]
    fn deduplicates_and_settles_before_deadline() {
        let mut state = Cohorts::default();
        let mut received = observation("ws_received", "9007199254740993", "received");
        state.observe(&mut received, 1);
        state.observe(&mut received, 2);
        state.observe(
            &mut observation("persist_terminal", "9007199254740993", "failed"),
            3,
        );
        state.observe(
            &mut observation("persist_terminal", "9007199254740993", "success"),
            4,
        );
        state.observe(
            &mut observation("persist_terminal", "9007199254740993", "failed"),
            5,
        );
        let closed = state.close_ready(2 * WINDOW_NS, false);
        assert_eq!(closed.len(), 1);
        assert_eq!(closed[0]["committed"], 1);
        assert_eq!(closed[0]["observed"], 1);
        assert_eq!(closed[0]["evidence_complete"], true);
        assert!(state.close_ready(3 * WINDOW_NS, false).is_empty());
        assert_eq!(
            state.observe(&mut received, 3 * WINDOW_NS).unwrap()["reason"],
            "retired_sequence_ambiguous"
        );
    }
    /// 超时后的成功不改原SLO，关闭pending为unknown，缺键和丢弃禁止完整率。
    #[test]
    fn timeout_close_and_missing_identity_are_not_success() {
        let mut state = Cohorts::default();
        state.observe(&mut observation("ws_received", "1", "received"), 1);
        assert_eq!(
            state
                .observe(
                    &mut observation("persist_terminal", "1", "success"),
                    WINDOW_NS + 2
                )
                .unwrap()["event"],
            "late_terminal"
        );
        assert_eq!(state.close_ready(2 * WINDOW_NS, false)[0]["timeout"], 1);
        state.observe(
            &mut observation("ws_received", "2", "received"),
            2 * WINDOW_NS,
        );
        state.mark_loss(true);
        let closed = state.close_ready(2 * WINDOW_NS + 1, true);
        assert_eq!(closed[0]["unknown"], 1);
        assert_eq!(closed[0]["evidence_complete"], false);
        let mut missing = observation("ws_received", "3", "received");
        missing.remove("user_id");
        assert_eq!(
            state.observe(&mut missing, 3 * WINDOW_NS).unwrap()["event"],
            "diagnostic_coverage_gap"
        );
        assert!(state.entries.is_empty());
    }
    /// 活动容量满时不淘汰旧义务、不制造成功率。
    #[test]
    fn bounded_capacity_keeps_existing_obligations() {
        let mut state = Cohorts::default();
        for seq in 1..=MAX_EVENTS {
            assert!(state
                .observe(
                    &mut observation("ws_received", &seq.to_string(), "received"),
                    1
                )
                .is_none());
        }
        assert_eq!(
            state
                .observe(&mut observation("ws_received", "99999", "received"), 1)
                .unwrap()["reason"],
            "capacity_limit"
        );
        assert_eq!(state.entries.len(), MAX_EVENTS);
        assert_eq!(
            state.close_ready(2 * WINDOW_NS, false)[0]["evidence_complete"],
            false
        );
    }
}
