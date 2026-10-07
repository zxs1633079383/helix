//! 失败来源在 Module 边界统一串联；只保存有界元数据，不保存 headers/body/业务对象。
//! 每个必达 corr 在回包时移除，timer 在触发/取消时移除，stop/身份切换清空。
use helix_core::effect::TimerId;
use helix_core::tick::PortOutcome;
use helix_core::{CoreError, Correlation, Effect, EffectSink, Tick};
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Weak};

#[derive(Default)]
pub(crate) struct FailureContext {
    pending: HashMap<Correlation, Source>,
    timers: HashMap<TimerId, Source>,
    requests: HashMap<Arc<str>, Weak<Origin>>,
    request_routes: HashMap<Arc<str>, Arc<str>>,
    current: Source,
    // HTTP 结束后的 WS 可晚到；诊断历史最多 256 项，不延长业务任务或保存 body。
    recent: HashMap<Arc<str>, Source>,
    recent_order: VecDeque<Arc<Origin>>,
}

#[derive(Default)]
struct Origin {
    command: Arc<str>,
    req_id: Arc<str>,
}

#[derive(Clone, Default)]
struct Source {
    origin: Option<Arc<Origin>>,
    route: Option<Arc<str>>,
    ws_action: Option<Cow<'static, str>>,
    stage: &'static str,
    corr: Option<u64>,
}

#[derive(Deserialize)]
struct RequestId<'a> {
    #[serde(borrow)]
    req_id: Option<Cow<'a, str>>,
    #[serde(borrow, rename = "requestId", alias = "reqId")]
    request_id: Option<Cow<'a, str>>,
}

#[derive(Deserialize)]
struct Envelope<'a> {
    #[serde(borrow)]
    event: Cow<'a, str>,
    #[serde(borrow)]
    data: &'a serde_json::value::RawValue,
}

#[derive(Deserialize)]
struct FailureFields<'a> {
    #[serde(borrow)]
    error: Option<Cow<'a, str>>,
    #[serde(borrow)]
    message: Option<Cow<'a, str>>,
    #[serde(borrow, rename = "failureCode")]
    failure_code: Option<Cow<'a, str>>,
}

impl FailureContext {
    /// O(1) 取本轮关联；owned 来源只在命令/I/O 注册时建立，不复制 wire payload。
    pub(crate) fn enter(&mut self, tick: &Tick) {
        self.current = match tick {
            Tick::Command(cmd) => {
                let id = serde_json::from_slice::<RequestId<'_>>(&cmd.payload)
                    .ok()
                    .and_then(|v| v.req_id.or(v.request_id));
                let origin = Arc::new(Origin {
                    command: Arc::from(clean_text(&cmd.name, 128)),
                    req_id: Arc::from(clean_text(id.as_deref().unwrap_or(""), 128)),
                });
                if !origin.req_id.is_empty() {
                    self.requests
                        .insert(Arc::clone(&origin.req_id), Arc::downgrade(&origin));
                }
                Source {
                    origin: Some(origin),
                    stage: "command",
                    ..Default::default()
                }
            }
            Tick::PortReply { corr, .. } => self.pending.remove(corr).unwrap_or_default(),
            Tick::PortProgress { corr, .. } => self.pending.get(corr).cloned().unwrap_or_default(),
            Tick::Timer(id) => {
                let mut source = self.timers.remove(id).unwrap_or_default();
                source.stage = "timer";
                source
            }
            _ => Source::default(),
        };
    }

    /// 使用已解析的 WS envelope；仅可信 tracing correlation 能关联已有 Command。
    pub(crate) fn websocket(&mut self, frame: &crate::ws::WsFrame) {
        let action = frame
            .root()
            .get("action")
            .or_else(|| frame.root().get("eventType"))
            .and_then(serde_json::Value::as_str)
            .or_else(|| frame.action().ok())
            .unwrap_or("unknown");
        let origin = frame.cses_track_id().and_then(|id| self.origin(id));
        let action = crate::ws::canonical_action(action)
            .map(Cow::Borrowed)
            .unwrap_or_else(|| Cow::Owned(clean_text(action, 128)));
        self.current = Source {
            origin,
            ws_action: Some(action),
            stage: "ws",
            ..Default::default()
        };
    }

    /// 只处理本 step 后缀；成功字节保持原样，失败才解析并重编码错误信封。
    pub(crate) fn finish(
        &mut self,
        tick: Option<&Tick>,
        start: usize,
        out: &mut EffectSink,
        result: Result<(), CoreError>,
    ) -> Result<(), CoreError> {
        let mut cause = None;
        let mut retired = Vec::new();
        out.retain_mut_from(start, |effect| {
            match effect {
                Effect::Http { corr, req } => {
                    let mut source = self.current.clone();
                    source.route = Some(Arc::from(format!(
                        "HTTP {} {}",
                        clean_text(&req.method, 16),
                        request_path(&req.url)
                    )));
                    source.stage = "http";
                    source.corr = Some(corr.raw());
                    if let (Some(origin), Some(route)) = (&source.origin, &source.route) {
                        if !origin.req_id.is_empty() {
                            self.request_routes
                                .insert(Arc::clone(&origin.req_id), Arc::clone(route));
                        }
                    }
                    self.pending.insert(*corr, source);
                }
                Effect::Persist { corr, .. }
                | Effect::PersistAtomic { corr, .. }
                | Effect::Request { corr, .. } => {
                    let mut source = self.current.clone();
                    source.stage = "port";
                    source.corr = Some(corr.raw());
                    self.pending.insert(*corr, source);
                }
                Effect::UploadFile { corr, req } => {
                    let mut source = self.current.clone();
                    source.stage = "upload";
                    source.corr = Some(corr.raw());
                    // Signed URL 和 local_path 不进入错误上下文。
                    source.route = Some(Arc::from(format!(
                        "HTTP {} [file-upload]",
                        clean_text(&req.method, 16)
                    )));
                    self.pending.insert(*corr, source);
                }
                Effect::ScheduleTimer { id, .. } => {
                    self.timers.insert(*id, self.current.clone());
                }
                Effect::CancelTimer { id } => {
                    if let Some(source) = self.timers.remove(id) {
                        retired.push(source);
                    }
                }
                Effect::Emit { event } => {
                    if !error_event_candidate(&event.0) {
                        return true;
                    }
                    let Ok(envelope) = serde_json::from_slice::<Envelope<'_>>(&event.0) else {
                        return true;
                    };
                    // 只检查错误出口；普通消息热路径不解析或序列化 payload。
                    if !matches!(
                        envelope.event.as_ref(),
                        "im:read:result"
                            | "im:posts:batch-result"
                            | "im:messages:query_result"
                            | "im:user:candidates"
                            | "im:operation:status"
                    ) && !envelope.event.ends_with("-failed")
                    {
                        return true;
                    }
                    let Ok(fields) = serde_json::from_str::<FailureFields<'_>>(envelope.data.get())
                    else {
                        return true;
                    };
                    let (field, message) = if let Some(error) = fields.error.as_deref() {
                        ("error", error)
                    } else if let Some(code) = fields.failure_code.as_deref() {
                        ("failureCode", code)
                    } else if envelope.event.ends_with("-failed") {
                        (
                            "message",
                            fields.message.as_deref().unwrap_or("operation failed"),
                        )
                    } else {
                        return true;
                    };
                    let remote = cause.get_or_insert_with(|| reply_cause(tick, self.current.stage));
                    let requested = serde_json::from_str::<RequestId<'_>>(envelope.data.get())
                        .ok()
                        .and_then(|id| id.req_id.or(id.request_id));
                    let mut source = self.current.clone();
                    if let Some(origin) = requested.as_deref().and_then(|id| self.origin(id)) {
                        source.origin = Some(origin);
                    }
                    if source.route.is_none() && source.ws_action.is_none() {
                        source.route =
                            self.request_routes
                                .get(source.req_id())
                                .cloned()
                                .or_else(|| {
                                    self.recent
                                        .get(source.req_id())
                                        .and_then(|s| s.route.clone())
                                });
                    }
                    let reason = match remote.as_deref() {
                        Some(remote) if !message.contains(remote) => format!("{message}; {remote}"),
                        _ => message.to_owned(),
                    };
                    let diagnostic = source.describe(&reason);
                    // 所有失败都有结构化日志；机器原因码不改写，避免破坏消费者分支。
                    tracing::warn!(command=source.command(), req_id=source.req_id(),
                        route=%RouteDisplay(&source), stage=source.stage,
                        corr=source.corr, error=%diagnostic,"IM operation failed");
                    // 固定的失败投影保持业务字段合同；来源写入统一结构化日志。
                    if envelope.event.ends_with("-failed") {
                        return true;
                    }
                    let Ok(mut data) =
                        serde_json::from_str::<serde_json::Value>(envelope.data.get())
                    else {
                        return true;
                    };
                    if is_reason_code(message)
                        || field == "failureCode"
                        || (field == "message" && envelope.event != "im:channel:create-failed")
                    {
                        data["errorContext"] = serde_json::Value::String(diagnostic);
                    } else {
                        data[field] = serde_json::Value::String(diagnostic);
                    }
                    #[derive(Serialize)]
                    struct ErrorEnvelope<'a> {
                        event: &'a str,
                        data: serde_json::Value,
                    }
                    if let Ok(encoded) = serde_json::to_vec(&ErrorEnvelope {
                        event: &envelope.event,
                        data,
                    }) {
                        event.0 = encoded.into();
                    }
                }
                _ => {}
            }
            true
        });
        let result = result.map_err(|error| CoreError::ModuleError {
            module: "helix-im",
            source: Box::new(ContextualFailure {
                context: self.current.describe(
                    reply_cause(tick, self.current.stage)
                        .as_deref()
                        .unwrap_or(&error.to_string()),
                ),
                source: error,
            }),
        });
        // 身份切换/取消是低频生命周期边界；清理不再等待的旧来源，不能串入新会话。
        if matches!(tick,Some(Tick::Command(cmd)) if cmd.name.as_ref()==crate::module::RUNTIME_IDENTITY_COMMAND)
        {
            self.pending.clear();
            self.timers.clear();
            self.requests.clear();
            self.request_routes.clear();
            self.recent.clear();
            self.recent_order.clear();
        }
        for source in retired {
            self.release(source);
        }
        let source = std::mem::take(&mut self.current);
        self.release(source);
        result
    }

    /// 按本次移除的来源释放弱索引，不对全体在途请求做热路径扫描。
    fn release(&mut self, source: Source) {
        if let Some(origin) = source.origin.as_ref() {
            if Arc::strong_count(origin) == 1
                && self
                    .requests
                    .get(origin.req_id.as_ref())
                    .is_some_and(|weak| weak.ptr_eq(&Arc::downgrade(origin)))
            {
                // 仅保留最近已释放来源；超限淘汰不影响业务，未知来源明确为 server_push。
                self.recent_order.push_back(Arc::clone(origin));
                self.recent
                    .insert(Arc::clone(&origin.req_id), source.clone());
                if self.recent_order.len() > 256 {
                    if let Some(old) = self.recent_order.pop_front() {
                        if self
                            .recent
                            .get(&old.req_id)
                            .and_then(|s| s.origin.as_ref())
                            .is_some_and(|current| Arc::ptr_eq(current, &old))
                        {
                            self.recent.remove(&old.req_id);
                        }
                    }
                }
                self.requests.remove(origin.req_id.as_ref());
                self.request_routes.remove(origin.req_id.as_ref());
            }
        }
    }

    /// 活跃请求优先；有界历史只提供来源，不能恢复已结束的业务请求。
    fn origin(&self, id: &str) -> Option<Arc<Origin>> {
        self.requests
            .get(id)
            .and_then(Weak::upgrade)
            .or_else(|| self.recent.get(id).and_then(|s| s.origin.clone()))
    }

    /// stop 时与业务状态一同释放诊断关联，不延长任何任务生命周期。
    pub(crate) fn clear(&mut self) {
        *self = Self::default();
    }
}

impl Source {
    /// 无关联主动推送明确标为 server_push，其他未知来源不伪造 Command。
    fn command(&self) -> &str {
        self.origin
            .as_ref()
            .map(|o| o.command.as_ref())
            .unwrap_or(if self.ws_action.is_some() {
                "server_push"
            } else {
                "internal"
            })
    }
    /// 外部请求 ID 仅取已登记的命令输入。
    fn req_id(&self) -> &str {
        self.origin
            .as_ref()
            .map(|o| o.req_id.as_ref())
            .unwrap_or("")
    }
    /// 统一人类可读错误格式；不包含 URL 查询、鉴权头或完整响应体。
    fn describe(&self, reason: &str) -> String {
        format!(
            "[command={} source={} stage={} req_id={} corr={}] {}",
            self.command(),
            RouteDisplay(self),
            self.stage,
            self.req_id(),
            self.corr
                .map(|c| c.to_string())
                .unwrap_or_else(|| "none".into()),
            clean_text(reason, 1024)
        )
    }
}

/// 格式化只发生在失败分支，成功 WS 不分配 route 字符串。
struct RouteDisplay<'a>(&'a Source);
impl std::fmt::Display for RouteDisplay<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(route) = &self.0.route {
            write!(f, "{route}")
        } else if let Some(action) = &self.0.ws_action {
            write!(f, "WS {action}")
        } else {
            f.write_str("local")
        }
    }
}

/// 项目 struct(event-first) / Value(event-last) 信封只查固定首尾，排除普通消息事件。
fn error_event_candidate(bytes: &[u8]) -> bool {
    let prefix = &bytes[..bytes.len().min(100)];
    let suffix = &bytes[bytes.len().saturating_sub(100)..];
    [
        b"im:read:result".as_slice(),
        b"im:posts:batch-result",
        b"im:messages:query_result",
        b"im:user:candidates",
        b"im:operation:status",
        b"-failed\"",
    ]
    .iter()
    .any(|name| {
        [prefix, suffix]
            .iter()
            .any(|part| part.windows(name.len()).any(|w| w == *name))
    })
}

#[derive(Debug, thiserror::Error)]
#[error("{context}")]
struct ContextualFailure {
    context: String,
    #[source]
    source: CoreError,
}

/// 只从实际失败 PortReply 提取原因，业务消息不从 HTTP200 推断成功。
fn reply_cause(tick: Option<&Tick>, stage: &str) -> Option<String> {
    let Some(Tick::PortReply { outcome, .. }) = tick else {
        return None;
    };
    match outcome {
        PortOutcome::Err(error) => Some(format!("port failure: {error:?}")),
        PortOutcome::Ok(reply) if stage == "http" => {
            crate::http_envelope::unwrap_sync_envelope(&reply.0)
                .err()
                .map(|e| e.to_string())
        }
        _ => None,
    }
}

/// 保留既有机器错误码，补充上下文另放 errorContext。
fn is_reason_code(message: &str) -> bool {
    !message.is_empty()
        && message
            .bytes()
            .all(|c| c.is_ascii_uppercase() || c == b'_' || c.is_ascii_digit())
}

/// 仅保留请求 path；移除 authority、userinfo、query 和 fragment。
fn request_path(url: &str) -> String {
    let path = if let Some((_, rest)) = url.split_once("://") {
        rest.find('/').map(|i| &rest[i..]).unwrap_or("/")
    } else {
        url
    };
    clean_text(path.split(['?', '#']).next().unwrap_or("/"), 256)
}

/// 外部错误只保留有限单行文本，并过滤常见凭据键值及 bearer 内容。
pub(crate) fn clean_text(text: &str, limit: usize) -> String {
    // 先限制输入，避免长错误文本在脱敏阶段产生无界临时分配。
    let bounded: String = text.chars().take(limit.saturating_add(128)).collect();
    let mut hide_next = false;
    bounded
        .split_whitespace()
        .map(|word| {
            let lower = word.to_ascii_lowercase();
            if hide_next {
                hide_next = false;
                return "[redacted]";
            }
            if matches!(
                lower.as_str(),
                "bearer" | "authorization:" | "cookie:" | "password:" | "token:"
            ) {
                hide_next = true;
                return "[redacted]";
            }
            if [
                "token=",
                "password=",
                "secret=",
                "authorization=",
                "cookie=",
            ]
            .iter()
            .any(|key| lower.contains(key))
            {
                return "[redacted]";
            }
            word
        })
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(limit)
        .collect()
}

#[cfg(test)]
#[path = "failure_context_tests.rs"]
mod tests;
