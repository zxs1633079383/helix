//! 定时媒体提交：复用发送域的票据、PUT 和稳定引用合同，上传完成后才创建待发对象。
//! 不写普通消息、不生成乐观时间线；中断只丢弃未提交意图，服务端到点执行权威不变。
use crate::module::ImModule;
use crate::send::upload_props::{self, PendingMediaPut, PlannedMedia, PreparedMedia};
use crate::state::{ChannelId, TemporaryId};
use crate::ImError;
use helix_core::effect::TimerId;
use helix_core::tick::{AppCommand, PortOutcome};
use helix_core::{Correlation, Effect, EffectSink, Module, Tick};
use serde_json::Value;
use std::collections::VecDeque;

#[derive(Debug)]
pub(crate) struct PendingScheduleMedia {
    payload: Value,
    remaining: VecDeque<PlannedMedia>,
    current: PlannedMedia,
    prepared: Option<PreparedMedia>,
    timer: TimerId,
}

impl ImModule {
    /// 有界 O(n) 规划附件；稳定引用无需上传，未知句柄在任何外部副作用前拒绝。
    pub(crate) fn begin_schedule_media(
        &mut self,
        bytes: &[u8],
        now_ms: u64,
        out: &mut EffectSink,
    ) -> Result<bool, ImError> {
        let mut payload: Value =
            serde_json::from_slice(bytes).map_err(|e| ImError::Parse(e.to_string()))?;
        // 先复用出站合同验证全部字段，禁止非法请求先上传后失败。
        crate::commands::handle_outbound(
            "im_create_schedule",
            bytes,
            &self.config.api_base_url,
            &self.config.default_api_base_url,
            self.state.connection_id.as_deref(),
            Correlation::from_raw(0),
        )?;
        let plan = upload_props::build_upload_plan(
            payload["type"].as_str().unwrap_or("TEXT"),
            payload["message"].as_str().unwrap_or(""),
            payload
                .get("props")
                .cloned()
                .unwrap_or_else(|| serde_json::json!({})),
        )?;
        if plan.media.is_empty() {
            return Ok(false);
        }
        if self.state.pending_schedule_media.len() >= 32 {
            return Err(ImError::Parse("too many pending schedule uploads".into()));
        }
        upload_props::validate_java_api_base_url(&self.config.default_api_base_url)?;
        payload["props"] = plan.props;
        let mut remaining: VecDeque<_> = plan.media.into();
        let current = remaining
            .pop_front()
            .ok_or_else(|| ImError::Parse("missing schedule media".into()))?;
        let timer = self.alloc_timer();
        self.dispatch_schedule_media(
            PendingScheduleMedia {
                payload,
                remaining,
                current,
                prepared: None,
                timer,
            },
            now_ms,
            out,
        )?;
        Ok(true)
    }

    /// 单附件单阶段出站，移动 payload 而非逐附件复制；超时只约束上传而非定时投递。
    fn dispatch_schedule_media(
        &mut self,
        pending: PendingScheduleMedia,
        _now_ms: u64,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let corr = self.alloc_corr_internal();
        let effect = if let Some(prepared) = &pending.prepared {
            Effect::UploadFile {
                corr,
                req: upload_props::media_upload_request(
                    &pending.current.input,
                    prepared,
                    &pending.current.target,
                )?,
            }
        } else {
            let upload_id = format!(
                "schedule-{}-{}",
                pending.payload["req_id"].as_str().unwrap_or("local"),
                corr.raw()
            );
            Effect::Http {
                corr,
                req: upload_props::prepare_upload_request(
                    &self.config.default_api_base_url,
                    &upload_id,
                    &pending.current.input,
                    &pending.current.target,
                    crate::acl::sync_http_effects::session_auth_headers(
                        self.state.connection_id.as_deref(),
                    ),
                )?,
            }
        };
        out.push(Effect::ScheduleTimer {
            id: pending.timer,
            after_ms: 120_000,
        });
        self.state.pending_schedule_media.insert(corr, pending);
        out.push(effect);
        Ok(())
    }

    /// corr 单次 remove 保证迟到/重复回包不重建定时对象；任一上传失败立即释放当前意图。
    pub(crate) fn schedule_media_reply(
        &mut self,
        mut pending: PendingScheduleMedia,
        outcome: &PortOutcome,
        now_ms: u64,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        out.push(Effect::CancelTimer { id: pending.timer });
        let req_id = pending.payload["req_id"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        let result = (|| -> Result<(), ImError> {
            let reply = match outcome {
                PortOutcome::Ok(reply) => reply,
                PortOutcome::Err(_) => {
                    return Err(ImError::Parse("schedule media upload failed".into()))
                }
            };
            if let Some(prepared) = pending.prepared.take() {
                let channel_id = pending.payload["channel_id"]
                    .as_str()
                    .and_then(ChannelId::from_str)
                    .ok_or_else(|| ImError::Parse("invalid schedule channel".into()))?;
                let put = PendingMediaPut {
                    temporary_id: TemporaryId(req_id.clone()),
                    channel_id,
                    target: pending.current.target,
                    input: pending.current.input,
                    prepared,
                };
                upload_props::mark_media_complete(&mut pending.payload["props"], &put)?;
                if let Some(next) = pending.remaining.pop_front() {
                    pending.current = next;
                    return self.dispatch_schedule_media(pending, now_ms, out);
                }
                let bytes = serde_json::to_vec(&pending.payload)
                    .map_err(|e| ImError::Serialize(e.to_string()))?;
                // 再入公开命令处理器，复用 HTTP/correlation/WS/Persist 的唯一创建链。
                self.handle(
                    &Tick::Command(AppCommand::new("im_create_schedule", bytes)),
                    now_ms,
                    out,
                )
                .map_err(|e| ImError::Parse(e.to_string()))
            } else {
                pending.prepared = Some(
                    upload_props::parse_prepare_reply(
                        reply.0.as_ref(),
                        &pending.current.input,
                        &pending.current.target,
                    )
                    .map_err(ImError::Parse)?,
                );
                self.dispatch_schedule_media(pending, now_ms, out)
            }
        })();
        if result.is_err() {
            out.push(crate::read_relay::emit_read_error(
                &req_id,
                "SCHEDULE_MEDIA_UPLOAD_FAILED",
            ));
        }
        Ok(())
    }

    /// 停止时释放全部未提交上传，回收 timer 并结束请求等待者。
    pub(crate) fn stop_schedule_media(&mut self, out: &mut EffectSink) {
        for (_, pending) in self.state.pending_schedule_media.drain() {
            out.push(Effect::CancelTimer { id: pending.timer });
            out.push(crate::read_relay::emit_read_error(
                pending.payload["req_id"].as_str().unwrap_or_default(),
                "SCHEDULE_MEDIA_INTERRUPTED",
            ));
        }
    }

    /// 最多 32 个上传意图的有界超时回收；已完成或迟到 timer 无副作用。
    pub(crate) fn schedule_media_timeout(&mut self, id: TimerId, out: &mut EffectSink) -> bool {
        let corr = self
            .state
            .pending_schedule_media
            .iter()
            .find_map(|(corr, pending)| (pending.timer == id).then_some(*corr));
        if let Some(pending) = corr.and_then(|corr| self.state.pending_schedule_media.remove(&corr))
        {
            out.push(crate::read_relay::emit_read_error(
                pending.payload["req_id"].as_str().unwrap_or_default(),
                "SCHEDULE_MEDIA_UPLOAD_TIMEOUT",
            ));
            return true;
        }
        false
    }
}
