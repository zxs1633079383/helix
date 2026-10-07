//! 失败来源在 Module 边界统一串联；只保存有界元数据，不保存 headers/body/业务对象。
//! 每个必达 corr 在回包时移除，timer 在触发/取消时移除，stop/身份切换清空。
use helix_core::effect::TimerId;
use helix_core::tick::PortOutcome;
use helix_core::{CoreError, Correlation, Effect, EffectSink, Tick};
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, OnceLock, Weak};

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
    command: Cow<'static, str>,
    req_id: OnceLock<Arc<str>>,
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
    /// 不读取Command payload；仅共享命令名称与owned关联，不把借用带到下一轮。
    pub(crate) fn enter(&mut self, tick: &Tick) {
        self.current = match tick {
            Tick::Command(cmd) => {
                let origin = Arc::new(Origin {
                    command: if cmd.name.len() > 512 {
                        Cow::Owned(clean_text(&cmd.name, 128))
                    } else {
                        cmd.name.clone()
                    },
                    req_id: OnceLock::new(),
                });
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

    /// 借用业务解码器已有的关联字段；OnceLock 只初始化一次，已发出的分支共享同一身份。
    pub(crate) fn bind_request(&mut self, id: &str) {
        // 超长标识仍由业务合同处理，诊断索引不复制无界外部字符串。
        if id.is_empty() || id.len() > 512 {
            return;
        }
        if self.current.origin.is_none() {
            self.current.origin = self.origin(id);
        }
        if let Some(origin) = &self.current.origin {
            if origin.req_id.get().is_none() {
                let key = origin.req_id.get_or_init(|| Arc::from(id));
                self.requests
                    .insert(Arc::clone(key), Arc::downgrade(origin));
            }
        }
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

    /// 来源字段从已有 continuation 借用，仅在绑定时转成 owned；访问范围严格限于本轮后缀。
    pub(crate) fn finish<'a>(
        &mut self,
        tick: Option<&Tick>,
        start: usize,
        out: &mut EffectSink,
        result: Result<(), CoreError>,
        lookup: impl Fn(Correlation) -> Option<&'a str>,
    ) -> Result<(), CoreError> {
        let mut cause = None;
        let mut retired = Vec::new();
        out.visit_with_diagnostics_from(start, |effect, diagnostics| {
            if let Effect::Http { corr, .. }
            | Effect::Persist { corr, .. }
            | Effect::PersistAtomic { corr, .. }
            | Effect::Request { corr, .. }
            | Effect::UploadFile { corr, .. } = effect
            {
                if let Some(id) = lookup(*corr) {
                    self.bind_request(id);
                }
            }
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
                        if !origin.request_id().is_empty() {
                            self.request_routes.insert(
                                Arc::clone(origin.req_id.get().expect("registered request id")),
                                Arc::clone(route),
                            );
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
                    if !event.needs_observation() {
                        return;
                    }
                    let Ok(envelope) = serde_json::from_slice::<Envelope<'_>>(&event.0) else {
                        return;
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
                        return;
                    }
                    let Ok(fields) = serde_json::from_str::<FailureFields<'_>>(envelope.data.get())
                    else {
                        return;
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
                        return;
                    };
                    let remote = cause.get_or_insert_with(|| reply_cause(tick, self.current.stage));
                    let requested = serde_json::from_str::<RequestId<'_>>(envelope.data.get())
                        .ok()
                        .and_then(|id| id.req_id.or(id.request_id));
                    if let Some(id) = requested.as_deref() {
                        self.bind_request(id);
                    }
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
                    let mut diagnostic = source.describe(&reason);
                    append_trace_id(&mut diagnostic, diagnostics.trace_id());
                    diagnostics.report("im.operation.failed", &diagnostic);
                    // 所有失败都有结构化日志；机器原因码不改写，避免破坏消费者分支。
                    tracing::warn!(command=%clean_text(source.command(), 128), req_id=%clean_text(source.req_id(), 128),
                        route=%RouteDisplay(&source), stage=source.stage,
                        corr=source.corr, error=%diagnostic,"IM operation failed");
                    // 固定失败保留原因码；未启用追踪时连原始bytes也不改写。
                    if envelope.event.ends_with("-failed") && diagnostics.trace_id().is_none() {
                        return;
                    }
                    let Ok(mut data) =
                        serde_json::from_str::<serde_json::Value>(envelope.data.get())
                    else {
                        return;
                    };
                    if envelope.event.ends_with("-failed") {
                        // 固定投影不新增字段；只在现有message上附上可查询的TraceId。
                        let mut message = message.to_owned();
                        append_trace_id(&mut message, diagnostics.trace_id());
                        data["message"] = serde_json::Value::String(message);
                    } else if is_reason_code(message)
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
        });
        let result = result.map_err(|error| {
            let mut context = self.current.describe(
                reply_cause(tick, self.current.stage)
                    .as_deref()
                    .unwrap_or(&error.to_string()),
            );
            append_trace_id(&mut context, out.diagnostics().trace_id());
            out.diagnostics_mut().report("im.module.failed", &context);
            CoreError::ModuleError {
                module: "helix-im",
                source: Box::new(ContextualFailure {
                    context,
                    source: error,
                }),
            }
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
                    .get(origin.request_id())
                    .is_some_and(|weak| weak.ptr_eq(&Arc::downgrade(origin)))
            {
                // 仅保留最近已释放来源；超限淘汰不影响业务，未知来源明确为 server_push。
                self.recent_order.push_back(Arc::clone(origin));
                self.recent.insert(
                    Arc::clone(origin.req_id.get().expect("registered request id")),
                    source.clone(),
                );
                if self.recent_order.len() > 256 {
                    if let Some(old) = self.recent_order.pop_front() {
                        if self
                            .recent
                            .get(old.request_id())
                            .and_then(|s| s.origin.as_ref())
                            .is_some_and(|current| Arc::ptr_eq(current, &old))
                        {
                            self.recent.remove(old.request_id());
                        }
                    }
                }
                self.requests.remove(origin.request_id());
                self.request_routes.remove(origin.request_id());
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

impl Origin {
    /// 返回已绑定关联或空值；借用不跨 Tick，不产生临时字符串。
    fn request_id(&self) -> &str {
        self.req_id.get().map(AsRef::as_ref).unwrap_or("")
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
        self.origin.as_ref().map(|o| o.request_id()).unwrap_or("")
    }
    /// 统一人类可读错误格式；不包含 URL 查询、鉴权头或完整响应体。
    fn describe(&self, reason: &str) -> String {
        format!(
            "[command={} source={} stage={} req_id={} corr={}] {}",
            clean_text(self.command(), 128),
            RouteDisplay(self),
            self.stage,
            clean_text(self.req_id(), 128),
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

/// 序列化前按已经解析的业务字段分类；成功事件不再在 Module 出口读 JSON。
pub(crate) fn needs_observation(event: &str, data: &serde_json::Value) -> bool {
    event.ends_with("-failed")
        || match event {
            "im:read:result"
            | "im:posts:batch-result"
            | "im:messages:query_result"
            | "im:user:candidates" => data.get("error").is_some_and(serde_json::Value::is_string),
            "im:operation:status" => data
                .get("failureCode")
                .is_some_and(serde_json::Value::is_string),
            _ => false,
        }
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
    let end = text
        .char_indices()
        .nth(limit.saturating_add(128))
        .map(|(i, _)| i)
        .unwrap_or(text.len());
    let mut result = String::with_capacity(end.min(limit.saturating_mul(4)));
    let mut hide_next = false;
    let mut remaining = limit;
    for word in text[..end].split_whitespace() {
        let key = ["bearer", "authorization:", "cookie:", "password:", "token:"]
            .iter()
            .any(|key| word.eq_ignore_ascii_case(key));
        let value = [
            "token=",
            "password=",
            "secret=",
            "authorization=",
            "cookie=",
        ]
        .iter()
        .any(|key| {
            word.as_bytes()
                .windows(key.len())
                .any(|part| part.eq_ignore_ascii_case(key.as_bytes()))
        });
        let shown = if hide_next || key || value {
            "[redacted]"
        } else {
            word
        };
        hide_next = !hide_next && key;
        if !result.is_empty() && remaining > 0 {
            result.push(' ');
            remaining -= 1;
        }
        for c in shown.chars().take(remaining) {
            result.push(c);
            remaining -= 1;
        }
        if remaining == 0 {
            break;
        }
    }
    result
}

#[cfg(test)]
#[path = "failure_context_tests.rs"]
mod tests;

/// 只在错误字符串末尾附上宿主注入的合法标识；不读取线程全局状态。
fn append_trace_id(message: &mut String, trace_id: Option<helix_core::diagnostics::TraceId>) {
    if let Some(id) = trace_id {
        use std::fmt::Write;
        let _ = write!(message, " [trace_id={}]", id.as_str());
    }
}
