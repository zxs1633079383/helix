use crate::error::ImError;
use crate::module::ImModule;
use crate::pending_send::PendingSend;
use crate::state::{ChannelId, SendStatus, TemporaryId};
use helix_core::effect::HttpRequest;
use helix_core::{Effect, EffectSink};
use serde_json::json;

impl ImModule {
    /// 处理 im_send_message 命令（§4.5 发送链路）
    ///
    /// 固定先吐：
    /// 1. Persist{P1, 乐观落库 Sending}
    /// 2. Emit(im:post:sending)
    ///
    /// 普通文本会在同一步继续吐 `Http{H1, POST /posts/create}` + `ScheduleTimer{T1, 15s}`；
    /// rich/file 若存在上传门控，则先吐若干 `UploadFile`，等最后一个上传成功并真正发
    /// `posts/create` 时才 arm Post timer。
    pub(crate) fn handle_send_message(
        &mut self,
        payload: &[u8],
        now_ms: u64,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        // 解析命令载荷
        let cmd: serde_json::Value = serde_json::from_slice(payload)
            .map_err(|e| ImError::Parse(format!("im_send_message payload: {}", e)))?;

        let channel_id = cmd["channel_id"]
            .as_str()
            .and_then(ChannelId::from_str)
            .ok_or_else(|| ImError::Parse("missing/invalid channel_id".to_string()))?;
        let tmp_id_str = cmd["temporary_id"]
            .as_str()
            .ok_or_else(|| ImError::Parse("missing temporary_id".to_string()))?
            .to_string();
        let tmp_id = TemporaryId(tmp_id_str.clone());

        // ── buildMessageObject 编排（P1 下沉）──────────────────────────────────
        // 现网前端 service 的 wire body 组装下沉到 outbound::send_build（C1 纯渲染）。命令载荷
        // （viewers/mentions/props/topic_id/replied/text）+ 注入身份/Clock → camelCase body。
        // 空消息（trim 空 + props 空）→ None → 拦截不发（现网 validMessages.length===0 → return）。
        let identity = self.config.user_identity();
        let Some(mut body) = crate::outbound::send_build::build_from_command(
            &cmd,
            channel_id.as_str(),
            &tmp_id_str,
            now_ms,
            &identity,
        ) else {
            return Ok(());
        };
        let msg_type = body["type"].as_str().unwrap_or("TEXT").to_string();
        let upload_plan = crate::send::upload_props::build_upload_plan(
            msg_type.as_str(),
            body["message"].as_str().unwrap_or(""),
            body.get("props").cloned().unwrap_or_else(|| json!({})),
        )?;
        body["props"] = upload_plan.props.clone();

        // 落库用的纯文本（与 body.message 同源：2000 截断后）。
        let persisted_text = body["message"].as_str().unwrap_or("").to_string();
        let props_json = serde_json::to_string(&upload_plan.props)
            .map_err(|e| ImError::Serialize(e.to_string()))?;

        // 分配 corr / timer。timer 始终绑定这条 Post send，但 upload-gated 路径要等真正 emit
        // `posts/create` 时才 arm；否则上传耗时会误写 `send_status=unsend`。
        let p1_corr = self.alloc_corr_internal();
        let t1_timer = self.alloc_timer();

        // 1. Persist{P1, 乐观落库 status=Sending}（PK=temporary_id，见 schema::IM_SCHEMA）。
        out.push(Effect::Persist {
            corr: p1_corr,
            ops: vec![crate::pending_send::optimistic_message_persist_op(
                &tmp_id_str,
                channel_id.as_str(),
                persisted_text,
                msg_type.as_str(),
                props_json,
            )],
        });

        // 4. Emit(im:post:sending)（render-ready 乐观行：text/type 由 helix 吐·壳直绑不取 pendingText）。
        out.push(crate::acl::to_effect::emit_post_sending(
            channel_id,
            &tmp_id_str,
            body["message"].as_str().unwrap_or(""),
            body["type"].as_str().unwrap_or("TEXT"),
        ));

        // 注册 PendingSend（等待 echo 对账）。connection_id 维度（M1）：记录本条在哪个握手
        // 会话下发，重连 increment 补回时按 temporaryId 去重不双份。
        let conn = self.state.connection_id.clone();
        let mut ps = PendingSend::new(tmp_id.clone(), t1_timer, conn);
        ps.status = SendStatus::Local;
        // R4-retain：回填本 send 的 P1 Persist corr，对账时按它 O(1) `corr_map.remove`，
        // 取代旧 `corr_map.retain` 全表扫描清 OptimisticSend 路由。
        ps.persist_corr = Some(p1_corr);
        if !upload_plan.targets.is_empty() {
            ps.body = Some(body.clone());
            ps.remaining_uploads = upload_plan.targets.len();
        }
        self.state.pending_sends.insert(tmp_id.clone(), ps);
        crate::send_reconcile::register_optimistic_send_corr(
            &mut self.state,
            p1_corr,
            tmp_id.clone(),
        );

        if upload_plan.targets.is_empty() {
            self.emit_posts_create_http(channel_id, tmp_id, &body, out)?;
        } else {
            for target in upload_plan.targets {
                let upload_corr = self.alloc_corr_internal();
                let req =
                    crate::send::upload_props::file_upload_request(&upload_plan.props, &target)?;
                self.state.pending_uploads.insert(
                    upload_corr,
                    crate::send::upload_props::PendingUpload {
                        temporary_id: tmp_id.clone(),
                        target,
                    },
                );
                out.push(Effect::UploadFile {
                    corr: upload_corr,
                    req,
                });
            }
        }

        Ok(())
    }

    pub(crate) fn emit_posts_create_http(
        &mut self,
        channel_id: ChannelId,
        temporary_id: TemporaryId,
        body: &serde_json::Value,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let h1_corr = self.alloc_corr_internal();
        let body_bytes = serde_json::to_vec(body).map_err(|e| ImError::Serialize(e.to_string()))?;
        out.push(Effect::Http {
            corr: h1_corr,
            req: HttpRequest {
                method: "POST".to_string(),
                url: format!("{}/posts/create", self.config.api_base_url),
                headers: {
                    let mut h = vec![("Content-Type".to_string(), "application/json".to_string())];
                    h.extend(crate::acl::to_effect::connection_id_headers(
                        self.state.connection_id.as_deref(),
                    ));
                    h
                },
                body: Some(bytes::Bytes::from(body_bytes)),
            },
        });
        let timeout_timer = self
            .state
            .pending_sends
            .get(&temporary_id)
            .map(|ps| ps.timeout_timer)
            .ok_or_else(|| {
                ImError::Parse(format!(
                    "missing pending send while emitting posts/create: {}",
                    temporary_id.0
                ))
            })?;
        out.push(Effect::ScheduleTimer {
            id: timeout_timer,
            after_ms: self.config.send_timeout_ms,
        });
        crate::send_reconcile::register_outbound_send_http_corr(
            &mut self.state,
            h1_corr,
            channel_id,
            temporary_id,
        );
        Ok(())
    }
}
