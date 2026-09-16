use crate::error::ImError;
use crate::module::ImModule;
use crate::pending_send::PendingSend;
use crate::state::{ChannelId, SendStatus, TemporaryId};
use helix_core::effect::HttpRequest;
use helix_core::{Effect, EffectSink};
use serde_json::{json, Value};

/// 递归定位仍未被 Host 解析的 opaque 媒体句柄，返回其 JSON 路径。
///
/// Host（`features/helix_im/media_stage.rs`）负责把 `mediaHandle` 换成账号绑定的
/// `mediaInput`；Core 侧只要还看得见 handle，就说明该 payload 没走过 Host 解析。
fn residual_media_handle(props: &Value) -> Option<String> {
    fn walk(value: &Value, path: &str, found: &mut Option<String>) {
        if found.is_some() {
            return;
        }
        match value {
            Value::Object(object) => {
                for (key, child) in object {
                    if key == "mediaHandle" || key == "media_handle" {
                        *found = Some(format!("{path}.{key}"));
                        return;
                    }
                    walk(child, &format!("{path}.{key}"), found);
                    if found.is_some() {
                        return;
                    }
                }
            }
            Value::Array(items) => {
                for (index, child) in items.iter().enumerate() {
                    walk(child, &format!("{path}[{index}]"), found);
                    if found.is_some() {
                        return;
                    }
                }
            }
            _ => {}
        }
    }
    let mut found = None;
    walk(props, "props", &mut found);
    found
}

impl ImModule {
    /// 处理 im_send_message 命令（§4.5 发送链路）
    ///
    /// 固定先吐：
    /// 1. Persist{P1, 乐观落库 Sending}
    /// 2. Emit(im:post:sending)
    ///
    /// 普通文本会先等待 P1 的已 attach timeline 读回产生真实事件，随后才吐
    /// `Http{H1, POST /posts/create}` + `ScheduleTimer{T1, 15s}`；rich/file 若存在
    /// 上传门控，首步只向 Java 发 `/oss/getPresignedUrl`；PUT 成功后停在 WaitingComplete，完整
    /// complete/posts.create 链由 G-03 B2 接续。
    pub(crate) fn handle_send_message(
        &mut self,
        payload: &[u8],
        now_ms: u64,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        // 解析命令载荷
        let cmd: serde_json::Value = serde_json::from_slice(payload)
            .map_err(|e| ImError::Parse(format!("im_send_message payload: {}", e)))?;

        // T073 / INV-01：opaque handle 活着进 Core ⇒ Host 的账号绑定解析被绕过。
        // 必须在任何状态变更 / Effect 之前 fail closed，且错误要点名具体 handle 路径，
        // 否则生产上媒体发送失败时无法定位是哪个文件的暂存记录丢了。
        if let Some(path) = cmd.get("props").and_then(residual_media_handle) {
            return Err(ImError::Parse(format!(
                "unresolved media handle at {path}: Host must resolve mediaHandle into mediaInput before Core"
            )));
        }

        let channel_id = cmd["channel_id"]
            .as_str()
            .and_then(ChannelId::from_str)
            .ok_or_else(|| ImError::Parse("missing/invalid channel_id".to_string()))?;
        // 乐观消息先进入内存业务事实，不能在 SQLite 写完成前让旧窗口 proof 命中假 Complete。
        self.state.invalidate_recent_message_coverage(channel_id);
        // public temporary_id 永不参与选择。实时命令由 Helix 铸造；仅确定性 replay/test
        // envelope 可携带已分配的 internal id，且公开 client_api 会删除该字段。
        let tmp_id = match cmd
            .get("allocated_temporary_id")
            .and_then(serde_json::Value::as_str)
            .filter(|value| !value.is_empty())
        {
            Some(allocated) => TemporaryId(allocated.to_string()),
            None => self.alloc_temporary_id(now_ms)?,
        };
        let tmp_id_str = tmp_id.0.clone();

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
        if !upload_plan.media.is_empty() {
            // 在写入 optimistic Sending 之前 fail-fast；否则缺少前端 Java endpoint 会让
            // prepare 构造错误被异步 engine 记录后丢弃，消息永久停在转圈状态。
            crate::send::upload_props::validate_java_api_base_url(
                &self.config.default_api_base_url,
            )?;
        }
        body["props"] = upload_plan.props.clone();

        // 分配 corr / timer。timer 始终绑定这条 Post send，但 upload-gated 路径要等真正 emit
        // `posts/create` 时才 arm；否则上传耗时会误写 `send_status=unsend`。
        let p1_corr = self.alloc_corr_internal();
        let t1_timer = self.alloc_timer();

        let planned_media = upload_plan
            .media
            .iter()
            .map(|media| crate::send::upload_props::PendingMediaPrepare {
                temporary_id: tmp_id.clone(),
                channel_id,
                target: media.target.clone(),
                input: media.input.clone(),
            })
            .collect::<Vec<_>>();
        // 1. 乐观落库 + 内部媒体恢复日志。媒体消息必须让 message 与 pending_media
        // 在同一事务提交；否则首个 StorageOp 成功、后一个失败后重启，会把未校验的
        // render props 当作普通消息直接发给 Go。普通文本仍保留既有 Persist 语义。
        let mut optimistic_ops = vec![crate::pending_send::optimistic_message_persist_op(
            &tmp_id_str,
            channel_id.as_str(),
            &body,
        )];
        if !planned_media.is_empty() {
            let operations = planned_media
                .iter()
                .cloned()
                .map(crate::send::upload_props::PendingMediaOp::Prepare)
                .collect::<Vec<_>>();
            optimistic_ops.push(crate::send::upload_props::durable_upsert_many(&operations)?);
        }
        if planned_media.is_empty() {
            out.push(Effect::Persist {
                corr: p1_corr,
                ops: optimistic_ops,
            });
        } else {
            out.push(Effect::PersistAtomic {
                corr: p1_corr,
                ops: optimistic_ops,
            });
        }

        // 注册 PendingSend（等待 echo 对账）。connection_id 维度（M1）：记录本条在哪个握手
        // 会话下发，重连 increment 补回时按 temporaryId 去重不双份。
        let conn = self.state.connection_id.clone();
        let mut ps = PendingSend::new(tmp_id.clone(), t1_timer, conn);
        ps.status = SendStatus::Local;
        ps.timeline_readback.causation_id = cmd
            .get("req_id")
            .and_then(serde_json::Value::as_str)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        // R4-retain：回填本 send 的 P1 Persist corr，对账时按它 O(1) `corr_map.remove`，
        // 取代旧 `corr_map.retain` 全表扫描清 OptimisticSend 路由。
        ps.persist_corr = Some(p1_corr);
        ps.body = Some(body.clone());
        if !upload_plan.media.is_empty() {
            ps.remaining_uploads = upload_plan.media.len();
        }
        self.state.pending_sends.insert(tmp_id.clone(), ps);
        if !planned_media.is_empty() {
            self.state
                .pending_media_after_optimistic
                .insert(tmp_id.clone(), planned_media);
        }
        crate::send_reconcile::register_optimistic_send_corr(
            &mut self.state,
            p1_corr,
            tmp_id.clone(),
        );

        Ok(())
    }

    pub(crate) fn emit_media_prepare(
        &mut self,
        channel_id: ChannelId,
        temporary_id: TemporaryId,
        target: crate::send::upload_props::UploadTarget,
        input: crate::send::upload_props::MediaInput,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let corr = self.alloc_corr_internal();
        let client_upload_id =
            crate::send::upload_props::client_upload_id(temporary_id.0.as_str(), &target);
        let req = crate::send::upload_props::prepare_upload_request(
            &self.config.default_api_base_url,
            &client_upload_id,
            &input,
            &target,
            crate::acl::sync_http_effects::session_auth_headers(
                self.state.connection_id.as_deref(),
            ),
        )?;
        self.state.pending_media_ops.insert(
            corr,
            crate::send::upload_props::PendingMediaOp::Prepare(
                crate::send::upload_props::PendingMediaPrepare {
                    temporary_id,
                    channel_id,
                    target,
                    input,
                },
            ),
        );
        out.push(Effect::Http { corr, req });
        Ok(())
    }

    pub(crate) fn emit_posts_create_http(
        &mut self,
        channel_id: ChannelId,
        temporary_id: TemporaryId,
        body: &serde_json::Value,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let (timeout_timer, request_id) = {
            let pending = self
                .state
                .pending_sends
                .get_mut(&temporary_id)
                .ok_or_else(|| {
                    ImError::Parse(format!(
                        "missing pending send while emitting posts/create: {}",
                        temporary_id.0
                    ))
                })?;
            if pending.http_started {
                return Ok(());
            }
            pending.http_started = true;
            (
                pending.timeout_timer,
                pending.timeline_readback.causation_id.clone(),
            )
        };
        let h1_corr = self.alloc_corr_internal();
        let body_bytes = serde_json::to_vec(body).map_err(|e| ImError::Serialize(e.to_string()))?;
        out.push(Effect::Http {
            corr: h1_corr,
            req: HttpRequest {
                method: "POST".to_string(),
                url: format!("{}/posts/create", self.config.api_base_url),
                headers: {
                    let mut h = vec![("Content-Type".to_string(), "application/json".to_string())];
                    h.extend(crate::acl::sync_http_effects::session_auth_headers(
                        self.state.connection_id.as_deref(),
                    ));
                    if let Some(request_id) = request_id.as_deref() {
                        h.push(("Cses-Track-Id".to_string(), request_id.to_string()));
                    }
                    h
                },
                body: Some(bytes::Bytes::from(body_bytes)),
            },
        });
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
