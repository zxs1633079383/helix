//! 可见书签总数的会话缓存。首次/重连/五分钟过期时校准；写回按 Go 的实际 delta 更新。
//! 命令 im_bookmark_count {req_id} → 缓存或 HTTP → im:read:result；无持久化、无新事件。
//! HTTP/业务/解析/超时失败保留 unknown，三十秒退避；身份切换/stop 取消所有等待者。
//! corr_map 消费一次，重复/迟到回包不重计；校准与写入重叠时只接受写入结束后的新快照。
use crate::{error::ImError, module::ImModule, state::CorrelationContext};
use helix_core::{tick::PortOutcome, EffectSink};
use serde::Deserialize;
use serde_json::{json, Value};

pub const COMMAND: &str = "im_bookmark_count";
const CALIBRATION: &str = "bookmark-count-calibration";
const TTL: u64 = 300_000;

#[derive(Default)]
pub(crate) struct CountCache {
    total: Option<u64>,
    checked_at: u64,
    retry_after: u64,
    revision: u64,
    reading: Option<u64>,
    writes: usize,
    waiters: Vec<String>,
}
impl CountCache {
    pub(crate) fn invalidate(&mut self) {
        self.total = None;
        self.revision = self.revision.wrapping_add(1);
    }
    pub(crate) fn write_started(&mut self) {
        self.writes += 1;
        self.revision = self.revision.wrapping_add(1);
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CountRequest {
    req_id: String,
}

impl ImModule {
    pub(crate) fn bookmark_count_read(
        &mut self,
        payload: &[u8],
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let request: CountRequest =
            serde_json::from_slice(payload).map_err(|e| ImError::Parse(e.to_string()))?;
        if request.req_id.trim().is_empty()
            || self.config.auth_user_id.is_empty()
            || self.config.company_id.is_empty()
        {
            return Err(ImError::Parse(
                "bookmark count requires req_id and runtime identity".into(),
            ));
        }
        let now = self.diagnostics.now_ms;
        if self.bookmark_count.total.is_some()
            && now.saturating_sub(self.bookmark_count.checked_at) < TTL
        {
            self.emit_bookmark_count(&request.req_id, out);
            return Ok(());
        }
        if now < self.bookmark_count.retry_after {
            out.push(crate::read_relay::emit_read_error(
                &request.req_id,
                "BOOKMARK_COUNT_RETRY_LATER",
            ));
            return Ok(());
        }
        if self.bookmark_count.waiters.len() >= 64 {
            return Err(ImError::Parse("bookmark count queue full".into()));
        }
        if !self.bookmark_count.waiters.contains(&request.req_id) {
            self.bookmark_count.waiters.push(request.req_id);
        }
        self.start_bookmark_count(out)
    }

    fn emit_bookmark_count(&self, req_id: &str, out: &mut EffectSink) {
        out.push(crate::read_relay::emit_read_body(req_id, json!({"status":"SUCCESS","data":{
            "userId":self.config.auth_user_id,"companyId":self.config.company_id,"total":self.bookmark_count.total
        }})));
    }

    fn start_bookmark_count(&mut self, out: &mut EffectSink) -> Result<(), ImError> {
        if self.bookmark_count.reading.is_some()
            || self.bookmark_count.writes > 0
            || self.bookmark_count.waiters.is_empty()
        {
            return Ok(());
        }
        let corr = self.alloc_corr_internal();
        let effects = crate::commands::handle_outbound(
            COMMAND,
            br#"{"req_id":"bookmark-count-calibration"}"#,
            &self.config.api_base_url,
            &self.config.default_api_base_url,
            self.state.connection_id.as_deref(),
            corr,
        )?;
        self.bookmark_count.reading = Some(self.bookmark_count.revision);
        self.state.corr_map.insert(
            corr,
            CorrelationContext::OutboundReadReply {
                req_id: CALIBRATION.into(),
                command: COMMAND.into(),
                channel_id: None,
            },
        );
        for effect in effects {
            out.push(effect);
        }
        Ok(())
    }

    pub(crate) fn bookmark_count_reply(
        &mut self,
        outcome: &PortOutcome,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let revision = self.bookmark_count.reading.take();
        if revision != Some(self.bookmark_count.revision) || self.bookmark_count.writes > 0 {
            return self.start_bookmark_count(out);
        }
        let body = bookmark_body(outcome);
        let total = body
            .as_ref()
            .filter(|b| self.bookmark_scope_valid(b))
            .and_then(|b| b["data"]["total"].as_u64());
        self.bookmark_count.total = total;
        self.bookmark_count.checked_at = self.diagnostics.now_ms;
        if total.is_none() {
            self.bookmark_count.retry_after = self.diagnostics.now_ms.saturating_add(30_000);
        }
        for req in std::mem::take(&mut self.bookmark_count.waiters) {
            if total.is_some() {
                self.emit_bookmark_count(&req, out);
            } else {
                out.push(crate::read_relay::emit_read_error(
                    &req,
                    "BOOKMARK_COUNT_UNAVAILABLE",
                ));
            }
        }
        Ok(())
    }

    pub(crate) fn bookmark_write_reply(
        &mut self,
        command: &str,
        outcome: &PortOutcome,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        self.bookmark_count.writes = self.bookmark_count.writes.saturating_sub(1);
        self.bookmark_count.revision = self.bookmark_count.revision.wrapping_add(1);
        let body = bookmark_body(outcome);
        let delta = body
            .as_ref()
            .filter(|b| self.bookmark_scope_valid(b))
            .and_then(|b| {
                let changed = b["data"]["affectedCount"].as_u64()?;
                let delta = b["data"]["countDelta"].as_i64()?;
                (delta.unsigned_abs() <= changed
                    && if command == "im_bookmark_create" {
                        delta >= 0
                    } else {
                        delta <= 0
                    })
                .then_some(delta)
            });
        self.bookmark_count.total = self
            .bookmark_count
            .total
            .zip(delta)
            .and_then(|(count, delta)| count.checked_add_signed(delta));
        // 失败后下一次读立即校准；HTTP 旧版本缺字段也走同一降级，不凭 SUCCESS 猜增量。
        if delta.is_none() {
            self.bookmark_count.retry_after = 0;
        }
        self.start_bookmark_count(out)
    }

    fn bookmark_scope_valid(&self, body: &Value) -> bool {
        body["status"] == "SUCCESS"
            && body["data"]["userId"].as_str() == Some(self.config.auth_user_id.as_str())
            && body["data"]["companyId"].as_str() == Some(self.config.company_id.as_str())
    }

    pub(crate) fn cancel_bookmark_count(&mut self, out: &mut EffectSink) {
        for req in std::mem::take(&mut self.bookmark_count.waiters) {
            out.push(crate::read_relay::emit_read_error(&req, "CANCELLED"));
        }
        self.bookmark_count = Default::default();
    }
}
fn bookmark_body(outcome: &PortOutcome) -> Option<Value> {
    match outcome {
        PortOutcome::Ok(reply) => crate::http_envelope::unwrap_sync_envelope(reply.0.as_ref())
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok()),
        _ => None,
    }
}
