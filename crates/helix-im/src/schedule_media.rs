//! 定时媒体提交：复用发送域的票据、PUT 和稳定引用合同，上传完成后才创建待发对象。
//! 不写普通消息、不生成乐观时间线；中断只丢弃未提交意图，服务端到点执行权威不变。
use crate::module::ImModule;
use crate::send::upload_props::{
    self, MultipartPart, MultipartSession, MultipartSignedPart, PendingMediaPrepare,
    PendingMediaPut, PlannedMedia, PreparedMedia,
};
use crate::state::{ChannelId, TemporaryId};
use crate::ImError;
use helix_core::effect::{BatchDeleteSpec, SqlValue, StorageOp, TimerId, UpsertSpec};
use helix_core::tick::{AppCommand, PortOutcome};
use helix_core::{Correlation, Effect, EffectSink, Module, Tick};
use serde_json::Value;
use std::collections::{HashSet, VecDeque};

const SCHEDULE_INTENT_TABLE: &str = "schedule_media_intent";
const SCHEDULE_INTENT_STAGE_INTENT: &str = "intent";
const SCHEDULE_INTENT_STAGE_PARTS: &str = "parts";
const SCHEDULE_INTENT_STAGE_SIGN: &str = "sign";
const SCHEDULE_INTENT_STAGE_SUBMITTING: &str = "submitting";
pub(crate) const SCHEDULE_INTENT_STAGE_FAILED: &str = "failed";
pub(crate) const SCHEDULE_INTENT_STAGE_UNKNOWN: &str = "unknown";
const SCHEDULE_UPLOAD_TOTAL_TIMEOUT_MS: u64 = 300_000;

/// 定时 multipart 回包阶段。路由只保存 key/小型 part，不复制整个 payload/session。
#[derive(Debug, Clone)]
pub(crate) enum ScheduleMultipartRoute {
    IntentPersist,
    LegacyPrepare,
    LegacyPut,
    MultipartInit,
    MultipartParts,
    MultipartSign,
    MultipartPut {
        part: MultipartSignedPart,
    },
    MultipartComplete,
    StagePersist {
        kind: ScheduleMultipartPersist,
        next: ScheduleMultipartNext,
    },
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum ScheduleMultipartPersist {
    Init,
    Parts,
    Part { part_number: u32 },
    Stable,
    Failed,
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum ScheduleMultipartNext {
    Current,
    Parts,
    Sign,
    Complete,
    Stable,
    PartAck { part_number: u32 },
    Failed,
}

#[derive(Debug, Clone)]
pub(crate) struct PendingScheduleMultipart {
    pub(crate) key: String,
    pub(crate) route: ScheduleMultipartRoute,
}

#[derive(Debug, Clone)]
pub(crate) struct ScheduleMultipartBatch {
    pub(crate) session: MultipartSession,
    pub(crate) signed: VecDeque<MultipartSignedPart>,
    pub(crate) inflight: HashSet<u32>,
}

/// 只保存 schedule owner/context；每个 part corr 通过 key 回到这里，避免复制完整 payload。
#[derive(Debug, Clone)]
pub(crate) struct ScheduleMultipartContext {
    pub(crate) key: String,
    /// Stable per-attachment identity. It survives a new UI request id so a
    /// retry can reuse the same Java session, while different files cannot
    /// collide merely because they share a schedule channel.
    pub(crate) upload_identity: String,
    pub(crate) payload: Value,
    pub(crate) remaining: VecDeque<PlannedMedia>,
    pub(crate) current: PlannedMedia,
    pub(crate) prepared: Option<PreparedMedia>,
    pub(crate) session: Option<MultipartSession>,
    /// Completed attachment session waiting for the next attachment's
    /// checkpoint.  It is deleted atomically before the next init is sent.
    pub(crate) retiring_session: Option<MultipartSession>,
    pub(crate) batch: Option<ScheduleMultipartBatch>,
    pub(crate) timer: TimerId,
    pub(crate) req_id: String,
    pub(crate) channel_id: ChannelId,
    pub(crate) schedule_post_at: i64,
    pub(crate) account_id: String,
    pub(crate) owner_user_id: String,
    pub(crate) last_error: String,
    pub(crate) pending_missing: Vec<u32>,
    /// A failed stage stops new work but keeps already-issued PUT/Persist
    /// routes alive until the drain barrier is empty.
    pub(crate) draining: bool,
    pub(crate) retry_attempts: u8,
    pub(crate) retry_after_ms: Option<u64>,
    /// Terminal failure is itself an atomic durable checkpoint. This guard
    /// prevents a late drain reply from enqueueing that checkpoint twice.
    pub(crate) terminal_failure_persisting: bool,
}

fn schedule_upload_identity(
    channel_id: ChannelId,
    schedule_post_at: i64,
    input: &upload_props::MediaInput,
    target: &upload_props::UploadTarget,
) -> String {
    let target_identity = match target {
        upload_props::UploadTarget::RichImage { index } => format!("rich:{index}"),
        upload_props::UploadTarget::RichVideo { index } => format!("rich-video:{index}"),
        upload_props::UploadTarget::File => "file".to_string(),
        upload_props::UploadTarget::TemplateImage => "template-image".to_string(),
    };
    upload_props::client_upload_id(
        &format!(
            "schedule:{}:{}:{}:{}:{}",
            channel_id.as_str(),
            schedule_post_at,
            input.size,
            input.sha256,
            target_identity
        ),
        target,
    )
}

/// Identifies one submitted schedule generation across cancel authority and
/// restart recovery. A channel alone is insufficient because a newer upload
/// may be registered before an older cancel PersistOk arrives.
pub(crate) fn schedule_submission_token(upload_identity: &str, request_id: &str) -> String {
    format!("{upload_identity}:{request_id}")
}

fn schedule_intent_upsert(
    context: &ScheduleMultipartContext,
    stage: &str,
    session: Option<&MultipartSession>,
    stable_object_ref: &str,
) -> Result<StorageOp, ImError> {
    let payload_json = serde_json::to_string(&context.payload)
        .map_err(|error| ImError::Serialize(error.to_string()))?;
    let mut media = Vec::with_capacity(context.remaining.len() + 1);
    media.push(schedule_media_value(&context.current));
    media.extend(context.remaining.iter().map(schedule_media_value));
    let media_json =
        serde_json::to_string(&media).map_err(|error| ImError::Serialize(error.to_string()))?;
    let session_id = session
        .map(|session| session.client_upload_id.as_str())
        .unwrap_or_default();
    Ok(StorageOp::BatchUpsert(UpsertSpec::new(
        SCHEDULE_INTENT_TABLE,
        vec![vec![
            (
                "channel_id".to_string(),
                SqlValue::Text(context.channel_id.as_str().to_string()),
            ),
            (
                "request_id".to_string(),
                SqlValue::Text(context.req_id.clone()),
            ),
            (
                "account_id".to_string(),
                SqlValue::Text(context.account_id.clone()),
            ),
            (
                "owner_user_id".to_string(),
                SqlValue::Text(context.owner_user_id.clone()),
            ),
            (
                "schedule_post_at".to_string(),
                SqlValue::Integer(context.schedule_post_at),
            ),
            ("payload_json".to_string(), SqlValue::Text(payload_json)),
            ("media_json".to_string(), SqlValue::Text(media_json)),
            ("stage".to_string(), SqlValue::Text(stage.to_string())),
            (
                "session_id".to_string(),
                SqlValue::Text(session_id.to_string()),
            ),
            (
                "stable_object_ref".to_string(),
                SqlValue::Text(stable_object_ref.to_string()),
            ),
            (
                "last_error".to_string(),
                SqlValue::Text(context.last_error.clone()),
            ),
        ]],
        Some("channel_id"),
    )))
}

fn schedule_media_value(media: &PlannedMedia) -> Value {
    let target = match &media.target {
        upload_props::UploadTarget::RichImage { index } => {
            serde_json::json!({"kind":"rich_image", "index": index})
        }
        upload_props::UploadTarget::RichVideo { index } => {
            serde_json::json!({"kind":"rich_video", "index": index})
        }
        upload_props::UploadTarget::File => serde_json::json!({"kind":"file"}),
        upload_props::UploadTarget::TemplateImage => serde_json::json!({"kind":"template_image"}),
    };
    serde_json::json!({
        "target": target,
        "localPath": media.input.local_path,
        "fileName": media.input.file_name,
        "contentType": media.input.content_type,
        "size": media.input.size,
        "sha256": media.input.sha256,
    })
}

fn parse_schedule_media_value(value: &Value) -> Result<PlannedMedia, ImError> {
    let target = value
        .get("target")
        .ok_or_else(|| ImError::Parse("schedule media target missing".into()))?;
    let target = match target.get("kind").and_then(Value::as_str) {
        Some("rich_image") => upload_props::UploadTarget::RichImage {
            index: target
                .get("index")
                .and_then(Value::as_u64)
                .and_then(|index| usize::try_from(index).ok())
                .ok_or_else(|| ImError::Parse("schedule rich image index invalid".into()))?,
        },
        Some("rich_video") => upload_props::UploadTarget::RichVideo {
            index: target
                .get("index")
                .and_then(Value::as_u64)
                .and_then(|index| usize::try_from(index).ok())
                .ok_or_else(|| ImError::Parse("schedule rich video index invalid".into()))?,
        },
        Some("file") => upload_props::UploadTarget::File,
        Some("template_image") => upload_props::UploadTarget::TemplateImage,
        _ => return Err(ImError::Parse("schedule media target invalid".into())),
    };
    let input = upload_props::MediaInput {
        local_path: value
            .get("localPath")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| ImError::Parse("schedule media localPath invalid".into()))?
            .to_string(),
        file_name: value
            .get("fileName")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| ImError::Parse("schedule media fileName invalid".into()))?
            .to_string(),
        content_type: value
            .get("contentType")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| ImError::Parse("schedule media contentType invalid".into()))?
            .to_string(),
        size: value
            .get("size")
            .and_then(Value::as_u64)
            .filter(|size| (1..=upload_props::MAX_MEDIA_SIZE_BYTES).contains(size))
            .ok_or_else(|| ImError::Parse("schedule media size invalid".into()))?,
        sha256: value
            .get("sha256")
            .and_then(Value::as_str)
            .filter(|value| {
                value.len() == 64
                    && value
                        .bytes()
                        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            })
            .ok_or_else(|| ImError::Parse("schedule media sha256 invalid".into()))?
            .to_string(),
    };
    Ok(PlannedMedia { target, input })
}

pub(crate) fn schedule_intent_delete(channel_id: ChannelId) -> StorageOp {
    StorageOp::BatchDelete(BatchDeleteSpec {
        table: SCHEDULE_INTENT_TABLE,
        scope_col: "channel_id",
        scope_val: SqlValue::Text(channel_id.as_str().to_string()),
        key_col: "channel_id",
        key_vals: vec![SqlValue::Text(channel_id.as_str().to_string())],
    })
}

/// Update only the submission outcome. The payload/session identity remains
/// durable so an HTTP response loss cannot be mistaken for a fresh request.
pub(crate) fn schedule_intent_stage_update(
    channel_id: ChannelId,
    stage: &'static str,
    error: &str,
) -> StorageOp {
    StorageOp::BatchUpdate(helix_core::effect::BatchUpdateSpec {
        table: SCHEDULE_INTENT_TABLE,
        key_col: "channel_id",
        key_vals: vec![SqlValue::Text(channel_id.as_str().to_string())],
        patch: vec![
            ("stage".to_string(), SqlValue::Text(stage.to_string())),
            ("last_error".to_string(), SqlValue::Text(error.to_string())),
        ],
    })
}

#[derive(Debug)]
pub(crate) struct PendingScheduleMedia {
    payload: Value,
    remaining: VecDeque<PlannedMedia>,
    current: PlannedMedia,
    prepared: Option<PreparedMedia>,
    timer: TimerId,
}

impl ImModule {
    /// Rehydrate only trusted schedule intent metadata. A restart always
    /// starts with Java init/parts, so local sparse rows are never promoted to
    /// completed parts without an authoritative remote listing.
    pub(crate) fn rehydrate_schedule_media(
        &mut self,
        bytes: &[u8],
        now_ms: u64,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let mut rows: Vec<Value> = serde_json::from_slice(bytes)
            .map_err(|error| ImError::Parse(format!("schedule intent scan: {error}")))?;
        rows.sort_by(|left, right| {
            left.get("channel_id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .cmp(
                    right
                        .get("channel_id")
                        .and_then(Value::as_str)
                        .unwrap_or_default(),
                )
        });
        for row in rows {
            let Some(channel_id) = row
                .get("channel_id")
                .and_then(Value::as_str)
                .and_then(ChannelId::from_str)
            else {
                continue;
            };
            let request_id = row
                .get("request_id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let stage = row.get("stage").and_then(Value::as_str).unwrap_or_default();
            let scope_ok = !self.config.company_id.is_empty()
                && !self.config.auth_user_id.is_empty()
                && row.get("account_id").and_then(Value::as_str)
                    == Some(self.config.company_id.as_str())
                && row.get("owner_user_id").and_then(Value::as_str)
                    == Some(self.config.auth_user_id.as_str());
            // A scan can contain rows from another account/owner when the
            // shared store is reused. Foreign rows are strictly read-only:
            // do not mark them failed, emit their request id, or start I/O.
            if !scope_ok {
                continue;
            }
            let schedule_post_at = row
                .get("schedule_post_at")
                .and_then(Value::as_i64)
                .unwrap_or_default();
            if schedule_post_at <= i64::try_from(now_ms).unwrap_or(i64::MAX) {
                let reason = "SCHEDULE_MEDIA_EXPIRED";
                let corr = self.alloc_corr_internal();
                out.push(Effect::PersistAtomic {
                    corr,
                    ops: vec![schedule_intent_stage_update(
                        channel_id,
                        SCHEDULE_INTENT_STAGE_FAILED,
                        reason,
                    )],
                });
                if !request_id.is_empty() {
                    out.push(crate::read_relay::emit_read_error(&request_id, reason));
                }
                continue;
            }
            if stage == SCHEDULE_INTENT_STAGE_SUBMITTING || stage == SCHEDULE_INTENT_STAGE_UNKNOWN {
                if self.state.schedule_media_submitting.len() >= 32
                    && !self
                        .state
                        .schedule_media_submitting
                        .contains_key(&channel_id)
                {
                    // Keep excess durable rows for a later bounded resume;
                    // startup must not fan out unbounded submissions.
                    continue;
                }
                let submission_identity = row
                    .get("session_id")
                    .and_then(Value::as_str)
                    .filter(|value| !value.is_empty())
                    .map(str::to_owned)
                    .unwrap_or_else(|| format!("schedule:{}", channel_id.as_str()));
                self.state.schedule_media_submitting.insert(
                    channel_id,
                    schedule_submission_token(&submission_identity, &request_id),
                );
                continue;
            }
            if matches!(stage, SCHEDULE_INTENT_STAGE_FAILED) {
                continue;
            }
            let Some(payload) = row
                .get("payload_json")
                .and_then(Value::as_str)
                .and_then(|value| serde_json::from_str::<Value>(value).ok())
            else {
                let corr = self.alloc_corr_internal();
                out.push(Effect::PersistAtomic {
                    corr,
                    ops: vec![schedule_intent_stage_update(
                        channel_id,
                        SCHEDULE_INTENT_STAGE_FAILED,
                        "SCHEDULE_MEDIA_PAYLOAD_INVALID",
                    )],
                });
                if !request_id.is_empty() {
                    out.push(crate::read_relay::emit_read_error(
                        &request_id,
                        "SCHEDULE_MEDIA_PAYLOAD_INVALID",
                    ));
                }
                continue;
            };
            let Some(media_values) = row
                .get("media_json")
                .and_then(Value::as_str)
                .and_then(|value| serde_json::from_str::<Vec<Value>>(value).ok())
            else {
                let corr = self.alloc_corr_internal();
                out.push(Effect::PersistAtomic {
                    corr,
                    ops: vec![schedule_intent_stage_update(
                        channel_id,
                        SCHEDULE_INTENT_STAGE_FAILED,
                        "SCHEDULE_MEDIA_METADATA_INVALID",
                    )],
                });
                if !request_id.is_empty() {
                    out.push(crate::read_relay::emit_read_error(
                        &request_id,
                        "SCHEDULE_MEDIA_METADATA_INVALID",
                    ));
                }
                continue;
            };
            let mut media = match media_values
                .iter()
                .map(parse_schedule_media_value)
                .collect::<Result<Vec<_>, _>>()
            {
                Ok(media) => media,
                Err(_) => {
                    let corr = self.alloc_corr_internal();
                    out.push(Effect::PersistAtomic {
                        corr,
                        ops: vec![schedule_intent_stage_update(
                            channel_id,
                            SCHEDULE_INTENT_STAGE_FAILED,
                            "SCHEDULE_MEDIA_METADATA_INVALID",
                        )],
                    });
                    if !request_id.is_empty() {
                        out.push(crate::read_relay::emit_read_error(
                            &request_id,
                            "SCHEDULE_MEDIA_METADATA_INVALID",
                        ));
                    }
                    continue;
                }
            };
            let Some(current) = media.first().cloned() else {
                let corr = self.alloc_corr_internal();
                out.push(Effect::PersistAtomic {
                    corr,
                    ops: vec![schedule_intent_stage_update(
                        channel_id,
                        SCHEDULE_INTENT_STAGE_FAILED,
                        "SCHEDULE_MEDIA_METADATA_EMPTY",
                    )],
                });
                if !request_id.is_empty() {
                    out.push(crate::read_relay::emit_read_error(
                        &request_id,
                        "SCHEDULE_MEDIA_METADATA_EMPTY",
                    ));
                }
                continue;
            };
            let remaining = media.drain(1..).collect::<VecDeque<_>>();
            let key = format!("schedule:{}", channel_id.as_str());
            if self.state.schedule_multipart_contexts.contains_key(&key)
                || self.state.schedule_multipart_contexts.len() >= 32
            {
                // Preserve the durable row for a later bounded resume rather
                // than marking a valid intent failed just because the host is
                // at its startup capacity.
                continue;
            }
            let upload_identity = row
                .get("session_id")
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
                .unwrap_or_else(|| {
                    schedule_upload_identity(
                        channel_id,
                        schedule_post_at,
                        &current.input,
                        &current.target,
                    )
                });
            let context = ScheduleMultipartContext {
                key: key.clone(),
                upload_identity,
                payload,
                remaining,
                current,
                prepared: None,
                session: None,
                retiring_session: None,
                batch: None,
                timer: self.alloc_timer(),
                req_id: request_id,
                channel_id,
                schedule_post_at,
                account_id: self.config.company_id.clone(),
                owner_user_id: self.config.auth_user_id.clone(),
                last_error: String::new(),
                pending_missing: Vec::new(),
                draining: false,
                retry_attempts: 0,
                retry_after_ms: None,
                terminal_failure_persisting: false,
            };
            self.state
                .schedule_multipart_contexts
                .insert(key.clone(), context);
            self.dispatch_schedule_multipart_current(&key, out)?;
        }
        Ok(())
    }

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
        payload["props"] = plan.props;
        // 只要该 schedule 含一份新协议媒体，整条附件编排由独立 schedule owner 持有；
        // 全部 <=100MiB 的旧链继续走下方 legacy PUT，保持现有 wire/超时行为。
        if plan.media.iter().any(|media| media.input.uses_multipart()) {
            return self.begin_schedule_multipart(payload, plan.media, now_ms, out);
        }
        if self.state.pending_schedule_media.len() >= 32 {
            return Err(ImError::Parse("too many pending schedule uploads".into()));
        }
        upload_props::validate_java_api_base_url(&self.config.default_api_base_url)?;
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

    fn begin_schedule_multipart(
        &mut self,
        mut payload: Value,
        media: Vec<PlannedMedia>,
        now_ms: u64,
        out: &mut EffectSink,
    ) -> Result<bool, ImError> {
        if self.state.schedule_multipart_contexts.len() >= 32 {
            return Err(ImError::Parse("too many pending schedule uploads".into()));
        }
        let channel_id = payload["channel_id"]
            .as_str()
            .and_then(ChannelId::from_str)
            .ok_or_else(|| ImError::Parse("invalid schedule channel".into()))?;
        if self
            .state
            .schedule_multipart_contexts
            .values()
            .any(|pending| pending.channel_id == channel_id)
            || self.state.pending_schedule_media.values().any(|pending| {
                pending
                    .payload
                    .get("channel_id")
                    .and_then(Value::as_str)
                    .and_then(ChannelId::from_str)
                    == Some(channel_id)
            })
            || self
                .state
                .pending_schedule_requests
                .contains_key(&channel_id)
            || self
                .state
                .schedule_media_submitting
                .contains_key(&channel_id)
        {
            return Err(ImError::Parse(
                "schedule channel already has a pending intent".into(),
            ));
        }
        upload_props::validate_java_api_base_url(&self.config.default_api_base_url)?;
        let req_id = payload["req_id"].as_str().unwrap_or_default().to_owned();
        let schedule_post_at = payload["schedule_post_at"].as_i64().ok_or_else(|| {
            ImError::Parse("schedule_post_at must be an integer for multipart intent".into())
        })?;
        if schedule_post_at <= i64::try_from(now_ms).unwrap_or(i64::MAX) {
            return Err(ImError::Parse("schedule_post_at is already expired".into()));
        }
        let current = media
            .first()
            .cloned()
            .ok_or_else(|| ImError::Parse("missing schedule media".into()))?;
        let upload_identity = schedule_upload_identity(
            channel_id,
            schedule_post_at,
            &current.input,
            &current.target,
        );
        let remaining = media.into_iter().skip(1).collect::<VecDeque<_>>();
        let key = format!("schedule:{}", channel_id.as_str());
        payload["props"] = payload
            .get("props")
            .cloned()
            .unwrap_or_else(|| serde_json::json!({}));
        let context = ScheduleMultipartContext {
            key: key.clone(),
            upload_identity,
            payload,
            remaining,
            current,
            prepared: None,
            session: None,
            retiring_session: None,
            batch: None,
            timer: self.alloc_timer(),
            req_id,
            channel_id,
            schedule_post_at,
            account_id: self.config.company_id.clone(),
            owner_user_id: self.config.auth_user_id.clone(),
            last_error: String::new(),
            pending_missing: Vec::new(),
            draining: false,
            retry_attempts: 0,
            retry_after_ms: None,
            terminal_failure_persisting: false,
        };
        self.state
            .schedule_multipart_contexts
            .insert(key.clone(), context);
        let corr = self.alloc_corr_internal();
        out.push(Effect::PersistAtomic {
            corr,
            ops: vec![schedule_intent_upsert(
                self.state
                    .schedule_multipart_contexts
                    .get(&key)
                    .expect("schedule multipart context inserted"),
                SCHEDULE_INTENT_STAGE_INTENT,
                None,
                "",
            )?],
        });
        self.state.pending_schedule_multipart.insert(
            corr,
            PendingScheduleMultipart {
                key,
                route: ScheduleMultipartRoute::IntentPersist,
            },
        );
        Ok(true)
    }

    /// 独立于普通 message 的 multipart schedule 回包路由。
    pub(crate) fn schedule_multipart_reply(
        &mut self,
        pending: PendingScheduleMultipart,
        outcome: &PortOutcome,
        now_ms: u64,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let Some(context) = self
            .state
            .schedule_multipart_contexts
            .get(&pending.key)
            .cloned()
        else {
            // stop/身份切换后迟到回包只释放 transport，不得复活 schedule intent。
            return Ok(());
        };
        if context.draining {
            // The route was issued by the failed batch. Its reply is only a
            // drain barrier; it must never checkpoint a late ETag or launch
            // the retry batch.
            return self.finish_schedule_multipart_drain(&pending.key, out);
        }
        match pending.route {
            ScheduleMultipartRoute::IntentPersist => {
                if matches!(outcome, PortOutcome::Ok(_)) {
                    self.dispatch_schedule_multipart_current(&pending.key, out)?;
                } else {
                    self.fail_schedule_multipart(
                        &pending.key,
                        "SCHEDULE_MEDIA_PERSIST_FAILED",
                        out,
                    )?;
                }
            }
            ScheduleMultipartRoute::LegacyPrepare => match outcome {
                PortOutcome::Ok(reply) => match upload_props::parse_prepare_reply(
                    reply.0.as_ref(),
                    &context.current.input,
                    &context.current.target,
                ) {
                    Ok(prepared) => {
                        if let Some(ctx) =
                            self.state.schedule_multipart_contexts.get_mut(&pending.key)
                        {
                            ctx.prepared = Some(prepared);
                        }
                        self.dispatch_schedule_multipart_current(&pending.key, out)?;
                    }
                    Err(reason) => self.fail_schedule_multipart(
                        &pending.key,
                        &format!("SCHEDULE_MEDIA_PREPARE_FAILED:{reason}"),
                        out,
                    )?,
                },
                PortOutcome::Err(error) => self.fail_schedule_multipart(
                    &pending.key,
                    &format!("SCHEDULE_MEDIA_PREPARE_FAILED:{error:?}"),
                    out,
                )?,
            },
            ScheduleMultipartRoute::LegacyPut => {
                if matches!(outcome, PortOutcome::Ok(_)) {
                    self.arm_schedule_multipart_timer(&pending.key, out);
                    let Some(context) = self
                        .state
                        .schedule_multipart_contexts
                        .get_mut(&pending.key)
                        .cloned()
                    else {
                        return Ok(());
                    };
                    let prepared = context.prepared.clone().ok_or_else(|| {
                        ImError::Parse("schedule legacy prepared media missing".into())
                    })?;
                    upload_props::mark_media_complete(
                        &mut self
                            .state
                            .schedule_multipart_contexts
                            .get_mut(&pending.key)
                            .expect("schedule multipart context exists")
                            .payload["props"],
                        &PendingMediaPut {
                            temporary_id: TemporaryId(context.key.clone()),
                            channel_id: context.channel_id,
                            target: context.current.target.clone(),
                            input: context.current.input.clone(),
                            prepared,
                        },
                    )?;
                    self.advance_schedule_after_media(&pending.key, out)?;
                } else if let PortOutcome::Err(error) = outcome {
                    self.fail_schedule_multipart(
                        &pending.key,
                        &format!("SCHEDULE_MEDIA_UPLOAD_FAILED:{error:?}"),
                        out,
                    )?;
                }
            }
            ScheduleMultipartRoute::MultipartInit => match outcome {
                PortOutcome::Ok(reply) => {
                    self.arm_schedule_multipart_timer(&pending.key, out);
                    let prepare = self.schedule_prepare(&context);
                    match upload_props::parse_multipart_init_reply(reply.0.as_ref(), &prepare) {
                        Ok(mut session) => {
                            // `parse_multipart_init_reply` derives the
                            // ordinary send identity from temporary_id. A
                            // schedule context already owns its durable
                            // per-attachment clientUploadId, so retain that
                            // exact wire identity across retries/restarts.
                            session.client_upload_id = context.upload_identity.clone();
                            if let Some(ctx) =
                                self.state.schedule_multipart_contexts.get_mut(&pending.key)
                            {
                                ctx.session = Some(session);
                            }
                            self.persist_schedule_multipart_stage(
                                &pending.key,
                                ScheduleMultipartPersist::Init,
                                ScheduleMultipartNext::Parts,
                                None,
                                out,
                            )?;
                        }
                        Err(reason) => {
                            self.fail_schedule_multipart(
                                &pending.key,
                                &format!("SCHEDULE_MEDIA_INIT_FAILED:{reason}"),
                                out,
                            )?;
                        }
                    }
                }
                PortOutcome::Err(error) => {
                    self.begin_schedule_multipart_drain(
                        &pending.key,
                        &format!("SCHEDULE_MEDIA_INIT_FAILED:{error:?}"),
                        Some(error),
                        out,
                    )?;
                }
            },
            ScheduleMultipartRoute::MultipartParts => match outcome {
                PortOutcome::Ok(reply) => {
                    self.arm_schedule_multipart_timer(&pending.key, out);
                    let Some(session) = context.session.clone() else {
                        return self.fail_schedule_multipart(
                            &pending.key,
                            "SCHEDULE_MEDIA_SESSION_MISSING",
                            out,
                        );
                    };
                    match upload_props::parse_multipart_parts_reply(reply.0.as_ref(), &session) {
                        Ok((status, parts)) => {
                            let mut session = session;
                            session.status = status;
                            session.parts = parts;
                            let mut present = [false; 128];
                            for part in &session.parts {
                                present[part.part_number as usize - 1] = true;
                            }
                            let missing = (1..=session.input.part_count())
                                .filter(|number| !present[*number as usize - 1])
                                .collect::<Vec<_>>();
                            if let Some(ctx) =
                                self.state.schedule_multipart_contexts.get_mut(&pending.key)
                            {
                                ctx.session = Some(session.clone());
                                ctx.pending_missing.clear();
                            }
                            if session.status == "completed" || missing.is_empty() {
                                self.persist_schedule_multipart_stage(
                                    &pending.key,
                                    ScheduleMultipartPersist::Parts,
                                    ScheduleMultipartNext::Complete,
                                    None,
                                    out,
                                )?;
                            } else {
                                if let Some(ctx) =
                                    self.state.schedule_multipart_contexts.get_mut(&pending.key)
                                {
                                    ctx.last_error.clear();
                                    ctx.pending_missing = missing.clone();
                                }
                                self.persist_schedule_multipart_stage(
                                    &pending.key,
                                    ScheduleMultipartPersist::Parts,
                                    ScheduleMultipartNext::Sign,
                                    None,
                                    out,
                                )?;
                            }
                        }
                        Err(reason) => {
                            self.fail_schedule_multipart(
                                &pending.key,
                                &format!("SCHEDULE_MEDIA_PARTS_FAILED:{reason}"),
                                out,
                            )?;
                        }
                    }
                }
                PortOutcome::Err(error) => {
                    self.begin_schedule_multipart_drain(
                        &pending.key,
                        &format!("SCHEDULE_MEDIA_PARTS_FAILED:{error:?}"),
                        Some(error),
                        out,
                    )?;
                }
            },
            ScheduleMultipartRoute::MultipartSign => match outcome {
                PortOutcome::Ok(reply) => {
                    self.arm_schedule_multipart_timer(&pending.key, out);
                    let Some(session) = context.session.clone() else {
                        return self.fail_schedule_multipart(
                            &pending.key,
                            "SCHEDULE_MEDIA_SESSION_MISSING",
                            out,
                        );
                    };
                    // The missing set is durable in the session/list response; this route
                    // stores the bounded batch only in memory, like ordinary multipart sign.
                    let mut present = [false; 128];
                    for part in &session.parts {
                        present[part.part_number as usize - 1] = true;
                    }
                    let missing = (1..=session.input.part_count())
                        .filter(|number| !present[*number as usize - 1])
                        .take(8)
                        .collect::<Vec<_>>();
                    match upload_props::parse_multipart_sign_reply(
                        reply.0.as_ref(),
                        &session,
                        &missing,
                    ) {
                        Ok(signed) => {
                            self.start_schedule_multipart_batch(&pending.key, session, signed, out)?
                        }
                        Err(reason) => self.fail_schedule_multipart(
                            &pending.key,
                            &format!("SCHEDULE_MEDIA_SIGN_FAILED:{reason}"),
                            out,
                        )?,
                    }
                }
                PortOutcome::Err(error) => {
                    self.begin_schedule_multipart_drain(
                        &pending.key,
                        &format!("SCHEDULE_MEDIA_SIGN_FAILED:{error:?}"),
                        Some(error),
                        out,
                    )?;
                }
            },
            ScheduleMultipartRoute::MultipartPut { part } => {
                if matches!(outcome, PortOutcome::Ok(_)) {
                    self.arm_schedule_multipart_timer(&pending.key, out);
                }
                self.handle_schedule_multipart_put(&pending.key, part, outcome, out)?;
            }
            ScheduleMultipartRoute::MultipartComplete => match outcome {
                PortOutcome::Ok(reply) => {
                    self.arm_schedule_multipart_timer(&pending.key, out);
                    let Some(session) = context.session.clone() else {
                        return self.fail_schedule_multipart(
                            &pending.key,
                            "SCHEDULE_MEDIA_SESSION_MISSING",
                            out,
                        );
                    };
                    match upload_props::parse_multipart_complete_reply(reply.0.as_ref(), &session) {
                        Ok(prepared) => {
                            let completed_session = context.session.clone();
                            let next = if let Some(ctx) =
                                self.state.schedule_multipart_contexts.get_mut(&pending.key)
                            {
                                upload_props::mark_media_complete(
                                    &mut ctx.payload["props"],
                                    &PendingMediaPut {
                                        temporary_id: TemporaryId(ctx.key.clone()),
                                        channel_id: ctx.channel_id,
                                        target: ctx.current.target.clone(),
                                        input: ctx.current.input.clone(),
                                        prepared: prepared.clone(),
                                    },
                                )?;
                                if let Some(next) = ctx.remaining.pop_front() {
                                    ctx.current = next;
                                    ctx.upload_identity = schedule_upload_identity(
                                        ctx.channel_id,
                                        ctx.schedule_post_at,
                                        &ctx.current.input,
                                        &ctx.current.target,
                                    );
                                    ctx.prepared = None;
                                    ctx.retiring_session = completed_session;
                                    ctx.session = None;
                                    ctx.batch = None;
                                    ScheduleMultipartNext::Current
                                } else {
                                    ctx.prepared = Some(prepared);
                                    ScheduleMultipartNext::Stable
                                }
                            } else {
                                return Ok(());
                            };
                            self.persist_schedule_multipart_stage(
                                &pending.key,
                                ScheduleMultipartPersist::Stable,
                                next,
                                None,
                                out,
                            )?;
                        }
                        Err(reason) => self.fail_schedule_multipart(
                            &pending.key,
                            &format!("SCHEDULE_MEDIA_COMPLETE_FAILED:{reason}"),
                            out,
                        )?,
                    }
                }
                PortOutcome::Err(error) => self.begin_schedule_multipart_drain(
                    &pending.key,
                    &format!("SCHEDULE_MEDIA_COMPLETE_FAILED:{error:?}"),
                    Some(error),
                    out,
                )?,
            },
            ScheduleMultipartRoute::StagePersist { kind, next } => {
                self.handle_schedule_multipart_stage_persist(
                    &pending.key,
                    kind,
                    next,
                    outcome,
                    now_ms,
                    out,
                )?;
            }
        }
        Ok(())
    }

    fn schedule_prepare(&self, context: &ScheduleMultipartContext) -> PendingMediaPrepare {
        PendingMediaPrepare {
            temporary_id: TemporaryId(context.upload_identity.clone()),
            channel_id: context.channel_id,
            target: context.current.target.clone(),
            input: context.current.input.clone(),
        }
    }

    /// Re-arm the single upload watchdog whenever the pipeline makes progress.
    /// The host's UploadFile has its own per-request total/idle budgets; this
    /// timer only detects a stalled sans-IO pipeline and therefore must not
    /// become a fixed whole-file deadline for a 1GiB upload.
    fn arm_schedule_multipart_timer(&mut self, key: &str, out: &mut EffectSink) {
        let Some(context) = self.state.schedule_multipart_contexts.get(key) else {
            return;
        };
        if !context.draining {
            let old_timer = context.timer;
            let new_timer = self.alloc_timer();
            if let Some(context) = self.state.schedule_multipart_contexts.get_mut(key) {
                context.timer = new_timer;
            }
            out.push(Effect::CancelTimer { id: old_timer });
            out.push(Effect::ScheduleTimer {
                id: new_timer,
                after_ms: SCHEDULE_UPLOAD_TOTAL_TIMEOUT_MS,
            });
        }
    }

    fn dispatch_schedule_multipart_current(
        &mut self,
        key: &str,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let Some(context) = self.state.schedule_multipart_contexts.get(key).cloned() else {
            return Ok(());
        };
        self.arm_schedule_multipart_timer(key, out);
        if context.prepared.is_some() {
            let prepared = context
                .prepared
                .clone()
                .ok_or_else(|| ImError::Parse("schedule prepared media missing".into()))?;
            let req = upload_props::media_upload_request(
                &context.current.input,
                &prepared,
                &context.current.target,
            )?;
            let corr = self.alloc_corr_internal();
            self.state.pending_schedule_multipart.insert(
                corr,
                PendingScheduleMultipart {
                    key: key.to_owned(),
                    route: ScheduleMultipartRoute::LegacyPut,
                },
            );
            out.push(Effect::UploadFile { corr, req });
            return Ok(());
        }
        if context.current.input.uses_multipart() {
            let prepare = self.schedule_prepare(&context);
            let req = upload_props::multipart_init_request(
                &self.config.default_api_base_url,
                &context.upload_identity,
                &prepare.input,
                context.current.target.bucket(),
                crate::acl::sync_http_effects::session_auth_headers(
                    self.state.connection_id.as_deref(),
                ),
            )?;
            let corr = self.alloc_corr_internal();
            self.state.pending_schedule_multipart.insert(
                corr,
                PendingScheduleMultipart {
                    key: key.to_owned(),
                    route: ScheduleMultipartRoute::MultipartInit,
                },
            );
            out.push(Effect::Http { corr, req });
        } else {
            let req = upload_props::prepare_upload_request(
                &self.config.default_api_base_url,
                &format!("schedule-{}-{}", context.req_id, key),
                &context.current.input,
                &context.current.target,
                crate::acl::sync_http_effects::session_auth_headers(
                    self.state.connection_id.as_deref(),
                ),
            )?;
            let corr = self.alloc_corr_internal();
            self.state.pending_schedule_multipart.insert(
                corr,
                PendingScheduleMultipart {
                    key: key.to_owned(),
                    route: ScheduleMultipartRoute::LegacyPrepare,
                },
            );
            out.push(Effect::Http { corr, req });
        }
        Ok(())
    }

    fn advance_schedule_after_media(
        &mut self,
        key: &str,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let Some(next) = self
            .state
            .schedule_multipart_contexts
            .get_mut(key)
            .and_then(|context| context.remaining.pop_front())
        else {
            return self.persist_schedule_multipart_stage(
                key,
                ScheduleMultipartPersist::Stable,
                ScheduleMultipartNext::Stable,
                None,
                out,
            );
        };
        if let Some(context) = self.state.schedule_multipart_contexts.get_mut(key) {
            context.current = next;
            context.upload_identity = schedule_upload_identity(
                context.channel_id,
                context.schedule_post_at,
                &context.current.input,
                &context.current.target,
            );
            context.prepared = None;
            context.session = None;
            context.batch = None;
        }
        self.persist_schedule_multipart_stage(
            key,
            ScheduleMultipartPersist::Stable,
            ScheduleMultipartNext::Current,
            None,
            out,
        )
    }

    fn emit_schedule_multipart_parts(
        &mut self,
        key: &str,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let context = self
            .state
            .schedule_multipart_contexts
            .get(key)
            .cloned()
            .ok_or_else(|| ImError::Parse("schedule multipart context missing".into()))?;
        let session = context
            .session
            .ok_or_else(|| ImError::Parse("schedule multipart session missing".into()))?;
        self.arm_schedule_multipart_timer(key, out);
        let req = upload_props::multipart_parts_request(
            &self.config.default_api_base_url,
            &session,
            crate::acl::sync_http_effects::session_auth_headers(
                self.state.connection_id.as_deref(),
            ),
        )?;
        let corr = self.alloc_corr_internal();
        self.state.pending_schedule_multipart.insert(
            corr,
            PendingScheduleMultipart {
                key: key.to_owned(),
                route: ScheduleMultipartRoute::MultipartParts,
            },
        );
        out.push(Effect::Http { corr, req });
        Ok(())
    }

    fn emit_schedule_multipart_sign(
        &mut self,
        key: &str,
        missing: Vec<u32>,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let context = self
            .state
            .schedule_multipart_contexts
            .get(key)
            .cloned()
            .ok_or_else(|| ImError::Parse("schedule multipart context missing".into()))?;
        let session = context
            .session
            .ok_or_else(|| ImError::Parse("schedule multipart session missing".into()))?;
        self.arm_schedule_multipart_timer(key, out);
        let missing = missing.into_iter().take(8).collect::<Vec<_>>();
        let req = upload_props::multipart_sign_request(
            &self.config.default_api_base_url,
            &session,
            &missing,
            crate::acl::sync_http_effects::session_auth_headers(
                self.state.connection_id.as_deref(),
            ),
        )?;
        let corr = self.alloc_corr_internal();
        self.state.pending_schedule_multipart.insert(
            corr,
            PendingScheduleMultipart {
                key: key.to_owned(),
                route: ScheduleMultipartRoute::MultipartSign,
            },
        );
        out.push(Effect::Http { corr, req });
        Ok(())
    }

    fn start_schedule_multipart_batch(
        &mut self,
        key: &str,
        session: MultipartSession,
        signed: Vec<MultipartSignedPart>,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        if signed.is_empty() {
            return self.emit_schedule_multipart_parts(key, out);
        }
        if let Some(context) = self.state.schedule_multipart_contexts.get_mut(key) {
            context.batch = Some(ScheduleMultipartBatch {
                session,
                signed: signed.into_iter().collect(),
                inflight: HashSet::new(),
            });
        }
        self.emit_schedule_multipart_batch_slots(key, out)
    }

    fn emit_schedule_multipart_batch_slots(
        &mut self,
        key: &str,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let mut slots = Vec::new();
        if let Some(context) = self.state.schedule_multipart_contexts.get_mut(key) {
            if let Some(batch) = context.batch.as_mut() {
                while batch.inflight.len() < 3 {
                    let Some(part) = batch.signed.pop_front() else {
                        break;
                    };
                    batch.inflight.insert(part.part_number);
                    slots.push((batch.session.clone(), part));
                }
            }
        }
        if !slots.is_empty() {
            self.arm_schedule_multipart_timer(key, out);
        }
        for (session, part) in slots {
            let req = upload_props::multipart_upload_request(
                &session.input,
                &session,
                &part,
                &session.target,
            )?;
            let corr = self.alloc_corr_internal();
            self.state.pending_schedule_multipart.insert(
                corr,
                PendingScheduleMultipart {
                    key: key.to_owned(),
                    route: ScheduleMultipartRoute::MultipartPut { part: part.clone() },
                },
            );
            out.push(Effect::UploadFile { corr, req });
        }
        Ok(())
    }

    fn handle_schedule_multipart_put(
        &mut self,
        key: &str,
        part: MultipartSignedPart,
        outcome: &PortOutcome,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let PortOutcome::Ok(reply) = outcome else {
            let PortOutcome::Err(error) = outcome else {
                unreachable!("PortOutcome was checked above")
            };
            return self.begin_schedule_multipart_drain(
                key,
                &format!("SCHEDULE_MEDIA_UPLOAD_FAILED:{error:?}"),
                Some(error),
                out,
            );
        };
        let success = match upload_props::parse_upload_success_reply(reply.0.as_ref()) {
            Ok(success) => success,
            Err(reason) => {
                return self.fail_schedule_multipart(
                    key,
                    &format!("SCHEDULE_MEDIA_UPLOAD_FAILED:{reason}"),
                    out,
                )
            }
        };
        let Some(etag) = success.etag.filter(|etag| !etag.trim().is_empty()) else {
            return self.fail_schedule_multipart(key, "SCHEDULE_MEDIA_UPLOAD_ETAG_MISSING", out);
        };
        let checkpoint =
            {
                let context = self
                    .state
                    .schedule_multipart_contexts
                    .get_mut(key)
                    .ok_or_else(|| ImError::Parse("schedule multipart context missing".into()))?;
                let batch = context
                    .batch
                    .as_mut()
                    .ok_or_else(|| ImError::Parse("schedule multipart batch missing".into()))?;
                if success.object_key != batch.session.object_key {
                    Err(ImError::Parse(
                        "SCHEDULE_MEDIA_UPLOAD_OBJECT_KEY_MISMATCH".into(),
                    ))
                } else {
                    match batch
                        .session
                        .parts
                        .iter()
                        .find(|existing| existing.part_number == part.part_number)
                    {
                        Some(existing) if existing.etag != etag => Err(ImError::Parse(
                            "SCHEDULE_MEDIA_UPLOAD_DUPLICATE_ETAG".into(),
                        )),
                        Some(_) => Ok(None),
                        None => {
                            let size = batch.session.input.part_size(part.part_number).ok_or_else(
                                || ImError::Parse("schedule multipart part out of range".into()),
                            )?;
                            let checkpoint = MultipartPart {
                                part_number: part.part_number,
                                size,
                                etag,
                            };
                            batch.session.parts.push(checkpoint.clone());
                            batch.session.parts.sort_by_key(|part| part.part_number);
                            context.session = Some(batch.session.clone());
                            Ok(Some(checkpoint))
                        }
                    }
                }
            };
        let checkpoint = match checkpoint {
            Ok(checkpoint) => checkpoint,
            Err(reason) => {
                return self.fail_schedule_multipart(
                    key,
                    &format!("SCHEDULE_MEDIA_UPLOAD_FAILED:{reason}"),
                    out,
                )
            }
        };
        if let Some(checkpoint) = checkpoint {
            self.persist_schedule_multipart_stage(
                key,
                ScheduleMultipartPersist::Part {
                    part_number: checkpoint.part_number,
                },
                ScheduleMultipartNext::PartAck {
                    part_number: checkpoint.part_number,
                },
                Some(vec![checkpoint]),
                out,
            )?;
        } else {
            self.schedule_multipart_part_ack(key, part.part_number, out)?;
        }
        Ok(())
    }

    fn schedule_multipart_part_ack(
        &mut self,
        key: &str,
        part_number: u32,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        if let Some(context) = self.state.schedule_multipart_contexts.get_mut(key) {
            if let Some(batch) = context.batch.as_mut() {
                batch.inflight.remove(&part_number);
            }
        }
        let should_requery = self
            .state
            .schedule_multipart_contexts
            .get(key)
            .and_then(|context| context.batch.as_ref())
            .is_some_and(|batch| batch.inflight.is_empty() && batch.signed.is_empty());
        if should_requery {
            if let Some(context) = self.state.schedule_multipart_contexts.get_mut(key) {
                context.batch = None;
            }
            self.emit_schedule_multipart_parts(key, out)
        } else {
            self.emit_schedule_multipart_batch_slots(key, out)
        }
    }

    fn persist_schedule_multipart_stage(
        &mut self,
        key: &str,
        kind: ScheduleMultipartPersist,
        next: ScheduleMultipartNext,
        checkpoint: Option<Vec<MultipartPart>>,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let Some(context) = self.state.schedule_multipart_contexts.get(key).cloned() else {
            return Ok(());
        };
        let session = context.session.clone();
        let retiring_session = context.retiring_session.clone();
        let stage = match (kind, next) {
            (ScheduleMultipartPersist::Stable, ScheduleMultipartNext::Stable) => {
                SCHEDULE_INTENT_STAGE_SUBMITTING
            }
            (_, ScheduleMultipartNext::Failed) => SCHEDULE_INTENT_STAGE_FAILED,
            (ScheduleMultipartPersist::Parts, ScheduleMultipartNext::Sign) => {
                SCHEDULE_INTENT_STAGE_SIGN
            }
            _ => SCHEDULE_INTENT_STAGE_PARTS,
        };
        let stable_ref = if stage == SCHEDULE_INTENT_STAGE_SUBMITTING {
            context
                .prepared
                .as_ref()
                .map(|prepared| format!("{}:{}", prepared.bucket, prepared.uri))
                .unwrap_or_default()
        } else {
            String::new()
        };
        let mut ops = if matches!(kind, ScheduleMultipartPersist::Part { .. }) {
            Vec::new()
        } else {
            vec![schedule_intent_upsert(
                &context,
                stage,
                session.as_ref(),
                &stable_ref,
            )?]
        };
        if let Some(session) = session.as_ref() {
            if matches!(kind, ScheduleMultipartPersist::Stable)
                && matches!(next, ScheduleMultipartNext::Stable)
            {
                ops.extend(upload_props::durable_multipart_delete_ops(session));
            } else if matches!(kind, ScheduleMultipartPersist::Failed) {
                ops.extend(upload_props::durable_multipart_delete_ops(session));
            } else if let ScheduleMultipartPersist::Part { part_number } = kind {
                let part = checkpoint
                    .as_ref()
                    .and_then(|parts| parts.iter().find(|part| part.part_number == part_number))
                    .or_else(|| {
                        session
                            .parts
                            .iter()
                            .find(|part| part.part_number == part_number)
                    })
                    .ok_or_else(|| {
                        ImError::Parse("schedule multipart checkpoint missing".into())
                    })?;
                // 每个确认 ETag 只写一个 sparse PK；不重写 session 或 schedule payload。
                ops.push(upload_props::durable_multipart_part_upsert_op(
                    session, part,
                ));
            } else {
                ops.push(upload_props::durable_multipart_session_upsert_op(session));
                if matches!(kind, ScheduleMultipartPersist::Parts) {
                    ops.push(upload_props::durable_multipart_parts_delete_op(session));
                    if let Some(parts) = upload_props::durable_multipart_parts_upsert_op(session) {
                        ops.push(parts);
                    }
                }
            }
        }
        // When one attachment completes and another remains, `session` has
        // already switched to the next attachment.  Carry the old session
        // separately so this same atomic checkpoint removes its sparse
        // session/part rows before the next init is allowed to run.
        if matches!(kind, ScheduleMultipartPersist::Stable)
            && matches!(next, ScheduleMultipartNext::Current)
        {
            if let Some(retiring_session) = retiring_session.as_ref() {
                ops.extend(upload_props::durable_multipart_delete_ops(retiring_session));
            }
        } else if matches!(kind, ScheduleMultipartPersist::Failed) {
            if let Some(retiring_session) = retiring_session.as_ref() {
                ops.extend(upload_props::durable_multipart_delete_ops(retiring_session));
            }
        }
        let corr = self.alloc_corr_internal();
        out.push(Effect::PersistAtomic { corr, ops });
        self.state.pending_schedule_multipart.insert(
            corr,
            PendingScheduleMultipart {
                key: key.to_owned(),
                route: ScheduleMultipartRoute::StagePersist { kind, next },
            },
        );
        Ok(())
    }

    fn handle_schedule_multipart_stage_persist(
        &mut self,
        key: &str,
        kind: ScheduleMultipartPersist,
        next: ScheduleMultipartNext,
        outcome: &PortOutcome,
        now_ms: u64,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        if let PortOutcome::Err(error) = outcome {
            if matches!(kind, ScheduleMultipartPersist::Failed) {
                if let Some(context) = self.state.schedule_multipart_contexts.get_mut(key) {
                    context.last_error = format!("SCHEDULE_MEDIA_PERSIST_FAILED:{error:?}");
                }
                return self.finish_schedule_multipart_failed(key, out);
            }
            return self.fail_schedule_multipart(
                key,
                &format!("SCHEDULE_MEDIA_PERSIST_FAILED:{error:?}"),
                out,
            );
        }
        self.arm_schedule_multipart_timer(key, out);
        if matches!(kind, ScheduleMultipartPersist::Stable)
            && matches!(next, ScheduleMultipartNext::Current)
        {
            if let Some(context) = self.state.schedule_multipart_contexts.get_mut(key) {
                context.retiring_session = None;
            }
        }
        match next {
            ScheduleMultipartNext::Current => self.dispatch_schedule_multipart_current(key, out),
            ScheduleMultipartNext::Parts => self.emit_schedule_multipart_parts(key, out),
            ScheduleMultipartNext::Sign => {
                let missing = self
                    .state
                    .schedule_multipart_contexts
                    .get(key)
                    .map(|context| context.pending_missing.clone())
                    .unwrap_or_default();
                self.emit_schedule_multipart_sign(key, missing, out)
            }
            ScheduleMultipartNext::Complete => self.emit_schedule_multipart_complete(key, out),
            ScheduleMultipartNext::Stable => self.submit_schedule_after_stable(key, now_ms, out),
            ScheduleMultipartNext::PartAck { part_number } => {
                self.schedule_multipart_part_ack(key, part_number, out)
            }
            ScheduleMultipartNext::Failed => self.finish_schedule_multipart_failed(key, out),
        }
    }

    fn emit_schedule_multipart_complete(
        &mut self,
        key: &str,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let context = self
            .state
            .schedule_multipart_contexts
            .get(key)
            .cloned()
            .ok_or_else(|| ImError::Parse("schedule multipart context missing".into()))?;
        let session = context
            .session
            .ok_or_else(|| ImError::Parse("schedule multipart session missing".into()))?;
        let req = upload_props::multipart_complete_request(
            &self.config.default_api_base_url,
            &session,
            crate::acl::sync_http_effects::session_auth_headers(
                self.state.connection_id.as_deref(),
            ),
        )?;
        let corr = self.alloc_corr_internal();
        self.state.pending_schedule_multipart.insert(
            corr,
            PendingScheduleMultipart {
                key: key.to_owned(),
                route: ScheduleMultipartRoute::MultipartComplete,
            },
        );
        out.push(Effect::Http { corr, req });
        Ok(())
    }

    fn submit_schedule_after_stable(
        &mut self,
        key: &str,
        now_ms: u64,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let context = self
            .state
            .schedule_multipart_contexts
            .get(key)
            .cloned()
            .ok_or_else(|| ImError::Parse("schedule multipart context missing".into()))?;
        if context.schedule_post_at <= i64::try_from(now_ms).unwrap_or(i64::MAX) {
            if let Some(context) = self.state.schedule_multipart_contexts.get_mut(key) {
                context.last_error = "SCHEDULE_MEDIA_EXPIRED".to_string();
            }
            return self.persist_schedule_multipart_stage(
                key,
                ScheduleMultipartPersist::Failed,
                ScheduleMultipartNext::Failed,
                None,
                out,
            );
        }
        let context = self
            .state
            .schedule_multipart_contexts
            .remove(key)
            .ok_or_else(|| ImError::Parse("schedule multipart context missing".into()))?;
        out.push(Effect::CancelTimer { id: context.timer });
        self.state.schedule_media_submitting.insert(
            context.channel_id,
            schedule_submission_token(&context.upload_identity, &context.req_id),
        );
        let bytes = serde_json::to_vec(&context.payload)
            .map_err(|error| ImError::Serialize(error.to_string()))?;
        // Re-enter the existing createSchedule route only after stage=submitting PersistOk;
        // this keeps TEXT wire behavior unchanged and prevents ordinary post creation.
        self.handle(
            &Tick::Command(AppCommand::new("im_create_schedule", bytes)),
            now_ms,
            out,
        )
        .map_err(|error| ImError::Parse(error.to_string()))
    }

    fn fail_schedule_multipart(
        &mut self,
        key: &str,
        error: &str,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        self.begin_schedule_multipart_drain(key, error, None, out)
    }

    fn finish_schedule_multipart_failed(
        &mut self,
        key: &str,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let Some(context) = self.state.schedule_multipart_contexts.remove(key) else {
            return Ok(());
        };
        out.push(Effect::CancelTimer { id: context.timer });
        if !context.req_id.is_empty() {
            out.push(crate::read_relay::emit_read_error(
                &context.req_id,
                &context.last_error,
            ));
        }
        Ok(())
    }

    fn begin_schedule_multipart_drain(
        &mut self,
        key: &str,
        error: &str,
        retryable: Option<&helix_core::tick::PortError>,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let Some(context) = self.state.schedule_multipart_contexts.get_mut(key) else {
            return Ok(());
        };
        if !context.draining {
            context.draining = true;
            context.last_error = error.to_string();
            context.batch = None;
            context.retry_after_ms = None;
            if let Some(port_error) = retryable {
                let retryable = matches!(
                    port_error,
                    helix_core::tick::PortError::Timeout
                        | helix_core::tick::PortError::Network
                        | helix_core::tick::PortError::Http(403 | 408 | 500..=599)
                );
                if retryable && context.retry_attempts < 3 {
                    context.retry_attempts += 1;
                    context.retry_after_ms = Some(
                        100_u64.saturating_mul(1_u64 << u32::from(context.retry_attempts - 1)),
                    );
                }
            }
            out.push(Effect::CancelTimer { id: context.timer });
        }
        self.finish_schedule_multipart_drain(key, out)
    }

    fn finish_schedule_multipart_drain(
        &mut self,
        key: &str,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        if self
            .state
            .pending_schedule_multipart
            .values()
            .any(|pending| pending.key == key)
        {
            return Ok(());
        }
        let Some(context) = self.state.schedule_multipart_contexts.get(key).cloned() else {
            return Ok(());
        };
        if let Some(after_ms) = context.retry_after_ms {
            let timer = self.alloc_timer();
            self.state
                .schedule_multipart_retry_timers
                .insert(timer, key.to_string());
            out.push(Effect::ScheduleTimer {
                id: timer,
                after_ms,
            });
            return Ok(());
        }
        if !context.terminal_failure_persisting {
            if let Some(context) = self.state.schedule_multipart_contexts.get_mut(key) {
                context.terminal_failure_persisting = true;
            }
            return self.persist_schedule_multipart_stage(
                key,
                ScheduleMultipartPersist::Failed,
                ScheduleMultipartNext::Failed,
                None,
                out,
            );
        }
        self.state.schedule_multipart_contexts.remove(key);
        self.state
            .pending_schedule_multipart
            .retain(|_, pending| pending.key != key);
        out.push(Effect::CancelTimer { id: context.timer });
        if !context.req_id.is_empty() {
            out.push(crate::read_relay::emit_read_error(
                &context.req_id,
                &context.last_error,
            ));
        }
        Ok(())
    }

    pub(crate) fn schedule_multipart_retry(
        &mut self,
        key: &str,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let Some(context) = self.state.schedule_multipart_contexts.get_mut(key) else {
            return Ok(());
        };
        if !context.draining {
            return Ok(());
        }
        context.draining = false;
        context.retry_after_ms = None;
        context.terminal_failure_persisting = false;
        context.session = None;
        context.batch = None;
        context.prepared = None;
        context.pending_missing.clear();
        self.dispatch_schedule_multipart_current(key, out)
    }

    pub(crate) fn schedule_multipart_timeout(
        &mut self,
        id: TimerId,
        now_ms: u64,
        out: &mut EffectSink,
    ) -> bool {
        let key = self
            .state
            .schedule_multipart_contexts
            .iter()
            .find_map(|(key, context)| (context.timer == id).then_some(key.clone()));
        if let Some(key) = key {
            let expired = self
                .state
                .schedule_multipart_contexts
                .get(&key)
                .is_some_and(|context| {
                    context.schedule_post_at <= i64::try_from(now_ms).unwrap_or(i64::MAX)
                });
            let _ = self.begin_schedule_multipart_drain(
                &key,
                if expired {
                    "SCHEDULE_MEDIA_EXPIRED"
                } else {
                    "SCHEDULE_MEDIA_UPLOAD_TIMEOUT"
                },
                (!expired).then_some(&helix_core::tick::PortError::Timeout),
                out,
            );
            return true;
        }
        false
    }

    pub(crate) fn stop_schedule_multipart(&mut self, out: &mut EffectSink) {
        let mut keys = self
            .state
            .schedule_multipart_contexts
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        keys.sort();
        for key in keys {
            let Some(context) = self.state.schedule_multipart_contexts.remove(&key) else {
                continue;
            };
            self.state
                .pending_schedule_multipart
                .retain(|_, pending| pending.key != key);
            out.push(Effect::CancelTimer { id: context.timer });
            if !context.req_id.is_empty() {
                out.push(crate::read_relay::emit_read_error(
                    &context.req_id,
                    "SCHEDULE_MEDIA_INTERRUPTED",
                ));
            }
        }
        self.state.pending_schedule_multipart.clear();
        let mut retry_timers = self
            .state
            .schedule_multipart_retry_timers
            .drain()
            .map(|(timer, _)| timer)
            .collect::<Vec<_>>();
        retry_timers.sort_by_key(|timer| timer.raw());
        for timer in retry_timers {
            out.push(Effect::CancelTimer { id: timer });
        }
    }

    /// Release only the submission generation captured by cancel authority.
    /// Active multipart contexts and their PUT/Persist routes deliberately
    /// remain alive; an old PersistOk must not clear a newer fence.
    pub(crate) fn clear_schedule_submission_fence(
        &mut self,
        channel_id: ChannelId,
        expected_token: Option<&str>,
    ) {
        let Some(expected_token) = expected_token else {
            return;
        };
        if self
            .state
            .schedule_media_submitting
            .get(&channel_id)
            .is_some_and(|current| current == expected_token)
        {
            self.state.schedule_media_submitting.remove(&channel_id);
        }
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
