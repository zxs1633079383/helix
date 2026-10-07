use std::sync::Arc;

use bytes::Bytes;

use super::carrier::{CommandTraceQueue, TraceCarrier};
use crate::base64_encode;
use crate::lifecycle::{
    LifecycleCapability, LifecycleContext, LifecycleObservation, LifecycleStage, LifecycleStatus,
    LifecycleTraceSink,
};
use crate::otel::{HostOtelRuntime, TraceDirection};
use crate::storage::StorageTraceContext;
use helix_core::effect::{
    Correlation, DomainEventBytes, FileUploadRequest, HttpRequest, StorageOp, TransportId,
};
use helix_core::Tick;

pub trait TraceHooksImpl {
    fn on_tick_start(&self, tick: &Tick, carrier: Option<TraceCarrier>) -> TraceScope;
    fn on_storage_dispatch(&self, corr: Option<Correlation>, ops: &[StorageOp]);
    fn on_http_dispatch(&self, corr: Option<Correlation>, req: &mut HttpRequest);
    fn on_upload_dispatch(&self, corr: Option<Correlation>, req: &FileUploadRequest);
    fn on_ws_send(&self, transport: TransportId, frame: &mut Bytes);
    fn on_event_emit(&self, event: &DomainEventBytes);
}

pub struct TraceScope {
    end: Option<Box<dyn FnOnce() + Send>>,
    root: Option<crate::otel::HostSpanScope>,
    step: Option<crate::otel::HostSpanScope>,
}

impl TraceScope {
    pub fn noop() -> Self {
        Self {
            end: None,
            root: None,
            step: None,
        }
    }

    pub fn new(end: impl FnOnce() + Send + 'static) -> Self {
        Self {
            end: Some(Box::new(end)),
            root: None,
            step: None,
        }
    }

    pub fn with_guard<G>(guard: G) -> Self
    where
        G: Send + 'static,
    {
        Self::new(move || drop(guard))
    }

    /// 同步执行期间显式注入当前span身份；返回前将业务失败记入同一step/root。
    pub fn step<'a>(
        &mut self,
        shell: &'a mut helix_core::ExecutionShell,
        tick: Tick,
        now_ms: u64,
    ) -> Result<&'a [helix_core::Effect], helix_core::CoreError> {
        let trace_id = self
            .step
            .as_ref()
            .and_then(|span| span.trace_id())
            .and_then(helix_core::diagnostics::TraceId::from_hex);
        // 先结束返回切片的借用，检查诊断后再重新借用同一缓冲。
        let result = shell.step_with_trace(tick, now_ms, trace_id).map(|_| ());
        for failure in shell.diagnostics().failures() {
            self.record_failure(failure);
        }
        if let Err(error) = &result {
            if shell.diagnostics().failures().is_empty() {
                self.record_failure(&helix_core::diagnostics::FailureDiagnostic {
                    kind: "core.step.failed",
                    message: error.to_string(),
                });
            }
        }
        result.map(|_| shell.effects())
    }

    /// 更新正在执行的span，不创建脱离当前链路的新trace。
    fn record_failure(&mut self, failure: &helix_core::diagnostics::FailureDiagnostic) {
        if let Some(step) = self.step.as_mut() {
            step.record_failure(failure);
        }
        if let Some(root) = self.root.as_mut() {
            root.record_failure(failure);
        }
    }

    pub fn chain(self, other: Self) -> Self {
        Self::new(move || {
            drop(other);
            drop(self);
        })
    }
}

impl Drop for TraceScope {
    fn drop(&mut self) {
        // 子span先结束，root最后结束；仅使用局部owned作用域，不跨await持有entered guard。
        drop(self.step.take());
        drop(self.root.take());
        if let Some(end) = self.end.take() {
            end();
        }
    }
}

#[derive(Clone)]
pub struct TraceHooks {
    inner: Arc<dyn TraceHooksImpl + Send + Sync>,
    command_traces: Option<CommandTraceQueue>,
    otel: Option<HostOtelRuntime>,
    lifecycle_sink: Option<LifecycleTraceSink>,
}

impl TraceHooks {
    pub fn new(inner: impl TraceHooksImpl + Send + Sync + 'static) -> Self {
        Self {
            inner: Arc::new(inner),
            command_traces: None,
            otel: None,
            lifecycle_sink: None,
        }
    }

    pub fn noop() -> Self {
        Self::new(NoopTraceHooks)
    }

    pub fn noop_without_otel() -> Self {
        Self::noop()
    }

    pub fn with_command_traces(mut self, command_traces: CommandTraceQueue) -> Self {
        self.command_traces = Some(command_traces);
        self
    }

    pub fn with_otel(mut self, otel: HostOtelRuntime) -> Self {
        self.otel = if otel.is_enabled() { Some(otel) } else { None };
        self
    }

    /// 注入有界 lifecycle 观测出口；队列满时只丢诊断，不影响业务泵。
    pub fn with_lifecycle_sink(mut self, sink: LifecycleTraceSink) -> Self {
        self.lifecycle_sink = Some(sink);
        self
    }

    /// composition root / 测试可用的装配状态；不暴露 exporter 或业务数据。
    pub fn is_otel_enabled(&self) -> bool {
        self.otel.is_some()
    }

    /// 从 Tick 创建显式上下文；只有 Command 会消费 sidecar carrier。
    pub fn context_for_tick(
        &self,
        tick: &Tick,
        tick_id: u64,
        parent_tick_id: Option<u64>,
        inherited_carrier: Option<TraceCarrier>,
    ) -> LifecycleContext {
        let carrier = match tick {
            Tick::Command(_) => self
                .command_traces
                .as_ref()
                .and_then(CommandTraceQueue::pop_next),
            _ => inherited_carrier,
        };
        LifecycleContext::new(tick_id, parent_tick_id, carrier)
    }

    /// 兼容旧调用方：无父关联的 Tick 仍通过显式上下文进入，不保存全局 carrier。
    pub fn on_tick_start(&self, tick: &Tick) -> TraceScope {
        let context = self.context_for_tick(tick, 0, None, None);
        self.on_tick_start_with_context(tick, &context)
    }

    /// 在显式 Tick 上下文中开启 T1 root scope；scope 只负责关闭 span，不恢复 ambient 状态。
    pub fn on_tick_start_with_context(
        &self,
        tick: &Tick,
        context: &LifecycleContext,
    ) -> TraceScope {
        self.start_tick_with_context(tick, context).0
    }

    /// 创建 T1 根及其本地 core step，并优先复用已保存的内部 parent，避免跨 Tick 重新开 Trace。
    pub fn start_tick_with_context(
        &self,
        tick: &Tick,
        context: &LifecycleContext,
    ) -> (TraceScope, LifecycleContext) {
        let carrier = context.carrier().cloned();
        let span_parent = context.otel_parent().cloned();
        let scope = self.inner.on_tick_start(tick, carrier.clone());
        let Some(runtime) = self.otel.as_ref() else {
            return (scope, context.clone());
        };

        let mut root_attributes =
            lifecycle_attributes(context, LifecycleStage::T1, None, LifecycleStatus::Started);
        if let Tick::Command(command) = tick {
            root_attributes.push((
                "helix.command.name",
                bounded_command_name(command.name.as_ref()),
            ));
        }
        let root_scope = runtime.span_with_owned_name(
            tick_root_span_name(tick),
            TraceDirection::Internal,
            span_parent.as_ref(),
            root_attributes,
        );
        let root_child_parent = root_scope.child_carrier();
        let step_scope = runtime.span_with_attributes(
            "helix.core.step",
            TraceDirection::Internal,
            root_child_parent.as_ref(),
            vec![("helix.lifecycle.internal", "core_step".to_string())],
        );
        // core.step 是 T1 的内部观测节点；T2/T3/T4/T5 必须直接复用 T1 parent，保持同级。
        let updated_context = context.with_span_parent(root_child_parent);
        let traced_scope = TraceScope {
            end: Some(Box::new(move || drop(scope))),
            root: Some(root_scope),
            step: Some(step_scope),
        };

        // 入站 WebSocket 本身是 T3；T1 根使用稳定的 Tick 类型名称。
        if matches!(tick, Tick::Inbound(_)) {
            self.emit_lifecycle_status(
                &updated_context,
                LifecycleStage::T3,
                Some(LifecycleCapability::Ws),
                LifecycleStatus::Started,
            );
        }
        (traced_scope, updated_context)
    }

    /// 兼容无 context 的 storage 调用；carrier 缺省为 None，不读取任何全局状态。
    pub fn on_storage_dispatch(&self, corr: Option<Correlation>, ops: &[StorageOp]) {
        let context = LifecycleContext::new(0, None, None)
            .with_capability(LifecycleCapability::Persist, true);
        self.on_storage_dispatch_with_context(&context, corr, ops);
    }

    /// 在显式上下文中记录 T4 Persist dispatch。
    pub fn on_storage_dispatch_with_context(
        &self,
        context: &LifecycleContext,
        corr: Option<Correlation>,
        ops: &[StorageOp],
    ) {
        let _scope = self.otel.as_ref().and_then(|runtime| {
            (context.has_capability(LifecycleCapability::Persist) && !ops.is_empty()).then(|| {
                let mut attributes = lifecycle_attributes(
                    context,
                    LifecycleStage::T4,
                    Some(LifecycleCapability::Persist),
                    LifecycleStatus::Started,
                );
                if runtime.is_full_debug() {
                    attributes.push((
                        "helix.debug.storage_ops",
                        bounded_debug_string(&format!("{ops:?}")),
                    ));
                }
                runtime.span_with_attributes(
                    "helix.storage.persist",
                    TraceDirection::Internal,
                    context.otel_parent(),
                    attributes,
                )
            })
        });
        self.inner.on_storage_dispatch(corr, ops);
    }

    /// 兼容无 context 的 storage worker 取 trace；carrier 缺省为 None。
    pub fn storage_trace_context(&self) -> Option<StorageTraceContext> {
        let context = LifecycleContext::new(0, None, None)
            .with_capability(LifecycleCapability::Persist, true);
        self.storage_trace_context_with_context(&context)
    }

    /// 为 Persist worker 创建显式 trace context，避免 task-local carrier 反向污染 engine。
    pub fn storage_trace_context_with_context(
        &self,
        context: &LifecycleContext,
    ) -> Option<StorageTraceContext> {
        self.otel
            .as_ref()
            .filter(|_| context.has_capability(LifecycleCapability::Persist))
            .map(|runtime| StorageTraceContext::from_lifecycle(runtime.clone(), context))
    }

    /// 兼容无 context 的 HTTP dispatch；无 carrier 时只使用请求自带合法 headers。
    pub fn on_http_dispatch(&self, corr: Option<Correlation>, req: &mut HttpRequest) {
        let context =
            LifecycleContext::new(0, None, None).with_capability(LifecycleCapability::Http, true);
        self.on_http_dispatch_with_context(&context, corr, req);
    }

    /// 在显式上下文中记录 T2 HTTP dispatch，并仅对真实 HTTP effect 注入 traceparent。
    pub fn on_http_dispatch_with_context(
        &self,
        context: &LifecycleContext,
        corr: Option<Correlation>,
        req: &mut HttpRequest,
    ) {
        let active = context.carrier();
        let mut dispatch_parent = None;
        let dispatch_scope = self
            .otel
            .as_ref()
            .filter(|_| context.has_capability(LifecycleCapability::Http))
            .map(|runtime| {
                let mut attributes = lifecycle_attributes(
                    context,
                    LifecycleStage::T2,
                    Some(LifecycleCapability::Http),
                    LifecycleStatus::Started,
                );
                if runtime.is_full_debug() {
                    attributes.extend(full_debug_http_attributes(req));
                }
                let scope = runtime.span_with_attributes(
                    "helix.http.dispatch",
                    TraceDirection::Outbound,
                    context.otel_parent(),
                    attributes,
                );
                dispatch_parent = scope.child_carrier();
                scope
            });
        if let Some(traceparent) = dispatch_parent
            .as_ref()
            .and_then(|carrier| carrier.traceparent.clone())
            .or_else(|| active.and_then(|carrier| carrier.traceparent.clone()))
        {
            if !req
                .headers
                .iter()
                .any(|(name, _)| name.eq_ignore_ascii_case("traceparent"))
            {
                req.headers.push(("traceparent".to_string(), traceparent));
            }
        }
        let _scope = dispatch_scope;
        self.inner.on_http_dispatch(corr, req);
    }

    /// 兼容无 context 的上传 dispatch。
    pub fn on_upload_dispatch(&self, corr: Option<Correlation>, req: &FileUploadRequest) {
        let context =
            LifecycleContext::new(0, None, None).with_capability(LifecycleCapability::Http, true);
        self.on_upload_dispatch_with_context(&context, corr, req);
    }

    /// 在显式上下文中记录 T2 upload dispatch。
    pub fn on_upload_dispatch_with_context(
        &self,
        context: &LifecycleContext,
        corr: Option<Correlation>,
        req: &FileUploadRequest,
    ) {
        let _scope = self
            .otel
            .as_ref()
            .filter(|_| context.has_capability(LifecycleCapability::Http))
            .map(|runtime| {
                let mut attributes = lifecycle_attributes(
                    context,
                    LifecycleStage::T2,
                    Some(LifecycleCapability::Http),
                    LifecycleStatus::Started,
                );
                if runtime.is_full_debug() {
                    attributes.extend(full_debug_upload_attributes(req));
                }
                runtime.span_with_attributes(
                    "helix.upload.dispatch",
                    TraceDirection::Outbound,
                    context.otel_parent(),
                    attributes,
                )
            });
        self.inner.on_upload_dispatch(corr, req);
    }

    /// 兼容无 context 的 WS dispatch。
    pub fn on_ws_send(&self, transport: TransportId, frame: &mut Bytes) {
        let context =
            LifecycleContext::new(0, None, None).with_capability(LifecycleCapability::Ws, true);
        self.on_ws_send_with_context(&context, transport, frame);
    }

    /// 在显式上下文中记录真实 WS send；缺 transport 时由 dispatch 侧报告状态，不在此伪造 span。
    pub fn on_ws_send_with_context(
        &self,
        context: &LifecycleContext,
        transport: TransportId,
        frame: &mut Bytes,
    ) {
        let _scope = self
            .otel
            .as_ref()
            .filter(|_| context.has_capability(LifecycleCapability::Ws))
            .map(|runtime| {
                let mut attributes = lifecycle_attributes(
                    context,
                    LifecycleStage::T3,
                    Some(LifecycleCapability::Ws),
                    LifecycleStatus::Started,
                );
                if runtime.is_full_debug() {
                    attributes.push(("helix.debug.ws_frame_base64", bounded_debug_bytes(frame)));
                }
                runtime.span_with_attributes(
                    "helix.ws.send",
                    TraceDirection::Outbound,
                    context.otel_parent(),
                    attributes,
                )
            });
        self.inner.on_ws_send(transport, frame);
    }

    /// 兼容无 context 的事件发射 dispatch。
    pub fn on_event_emit(&self, event: &DomainEventBytes) {
        let context =
            LifecycleContext::new(0, None, None).with_capability(LifecycleCapability::Effect, true);
        self.on_event_emit_with_context(&context, event);
    }

    /// 在显式上下文中记录 T5 event emit。
    pub fn on_event_emit_with_context(&self, context: &LifecycleContext, event: &DomainEventBytes) {
        let _scope = self
            .otel
            .as_ref()
            .filter(|_| context.has_capability(LifecycleCapability::Effect))
            .map(|runtime| {
                let mut attributes = lifecycle_attributes(
                    context,
                    LifecycleStage::T5,
                    Some(LifecycleCapability::Effect),
                    LifecycleStatus::Started,
                );
                if runtime.is_full_debug() {
                    attributes.push(("helix.debug.event_base64", bounded_debug_bytes(&event.0)));
                }
                runtime.span_with_attributes(
                    "helix.event.emit",
                    TraceDirection::Outbound,
                    context.otel_parent(),
                    attributes,
                )
            });
        self.inner.on_event_emit(event);
    }

    /// 记录通用 T3 Effect dispatch；只写结构化状态，不携带 Effect payload。
    pub fn on_effect_dispatch_with_context(&self, context: &LifecycleContext, kind: &'static str) {
        let _scope = self
            .otel
            .as_ref()
            .filter(|_| context.has_capability(LifecycleCapability::Effect))
            .map(|runtime| {
                runtime.span_with_attributes(
                    "helix.effect.dispatch",
                    TraceDirection::Internal,
                    context.otel_parent(),
                    lifecycle_attributes(
                        context,
                        LifecycleStage::T5,
                        Some(LifecycleCapability::Effect),
                        LifecycleStatus::Started,
                    )
                    .into_iter()
                    .chain([("helix.effect.kind", kind.to_string())])
                    .collect(),
                )
            });
    }

    /// 发出 not_applicable/skipped/error 状态；该 seam 不创建 HTTP/WS 网络 span。
    pub fn emit_lifecycle_status(
        &self,
        context: &LifecycleContext,
        stage: LifecycleStage,
        capability: Option<LifecycleCapability>,
        status: LifecycleStatus,
    ) {
        self.emit_lifecycle_status_with_reason(context, stage, capability, status, None);
    }

    /// 发出带固定原因的生命周期状态；reason 只用于解释 capability 缺失或跳过。
    pub fn emit_lifecycle_status_with_reason(
        &self,
        context: &LifecycleContext,
        stage: LifecycleStage,
        capability: Option<LifecycleCapability>,
        status: LifecycleStatus,
        reason: Option<&'static str>,
    ) {
        if let Some(sink) = &self.lifecycle_sink {
            let _ = sink.try_emit(LifecycleObservation {
                tick_id: context.tick_id(),
                parent_tick_id: context.parent_tick_id(),
                stage,
                capability,
                status,
                reason,
            });
        }
        let _scope = self.otel.as_ref().map(|runtime| {
            let mut attributes = lifecycle_attributes(context, stage, capability, status);
            if let Some(reason) = reason {
                attributes.push(("helix.lifecycle.reason", reason.to_string()));
            }
            runtime.span_with_attributes(
                "helix.lifecycle.status",
                TraceDirection::Internal,
                context.otel_parent(),
                attributes,
            )
        });
    }
}

/// 生成有界 lifecycle span 属性；只使用 scalar correlation，不写业务正文。
fn lifecycle_attributes(
    context: &LifecycleContext,
    stage: LifecycleStage,
    capability: Option<LifecycleCapability>,
    status: LifecycleStatus,
) -> Vec<(&'static str, String)> {
    let mut attributes = vec![
        ("helix.lifecycle.stage", stage.as_str().to_string()),
        ("helix.lifecycle.status", status.as_str().to_string()),
        ("helix.tick_id", context.tick_id().to_string()),
        (
            "helix.parent_tick_id",
            context
                .parent_tick_id()
                .map_or_else(|| "none".to_string(), |value| value.to_string()),
        ),
    ];
    if let Some(capability) = capability {
        attributes.push((
            "helix.lifecycle.capability",
            capability.as_str().to_string(),
        ));
    }
    if stage != LifecycleStage::T1 {
        attributes.push((
            "helix.lifecycle.parent_stage",
            LifecycleStage::T1.as_str().to_string(),
        ));
    }
    attributes
}

const FULL_DEBUG_PAYLOAD_LIMIT: usize = 64 * 1024;
const COMMAND_NAME_LIMIT: usize = 256;

/// 为 T1 生成有界 Span 名称；命令名只来自受控 Tick，不携带业务 payload。
fn tick_root_span_name(tick: &Tick) -> String {
    match tick {
        Tick::Command(command) => {
            format!(
                "helix.command.{}",
                bounded_command_name(command.name.as_ref())
            )
        }
        Tick::Inbound(_) => "helix.tick.inbound".to_string(),
        Tick::PortReply { .. } => "helix.tick.port_reply".to_string(),
        Tick::PortProgress { .. } => "helix.tick.port_progress".to_string(),
        Tick::Timer(_) => "helix.tick.timer".to_string(),
        Tick::Connected(_) => "helix.tick.connected".to_string(),
        Tick::Disconnected(_) => "helix.tick.disconnected".to_string(),
    }
}

/// 将 command 名限制为低成本 Trace 属性，避免用户可控名称无限增长。
fn bounded_command_name(value: &str) -> String {
    value.chars().take(COMMAND_NAME_LIMIT).collect()
}

/// 将 full_debug 字符串限制在有界单条事件内，超限只保留前缀和长度标记。
fn bounded_debug_string(value: &str) -> String {
    if value.len() <= FULL_DEBUG_PAYLOAD_LIMIT {
        return value.to_string();
    }
    let boundary = value
        .char_indices()
        .map(|(index, _)| index)
        .take_while(|index| *index <= FULL_DEBUG_PAYLOAD_LIMIT)
        .last()
        .unwrap_or(0);
    format!(
        "{}...[truncated {} bytes]",
        &value[..boundary],
        value.len() - boundary
    )
}

/// full_debug 下记录 HTTP 的完整请求字段；safe 模式不调用该 helper。
fn full_debug_http_attributes(req: &HttpRequest) -> Vec<(&'static str, String)> {
    let headers = serde_json::to_string(&req.headers).unwrap_or_else(|_| "[]".to_string());
    let mut attributes = vec![
        ("helix.debug.http.method", bounded_debug_string(&req.method)),
        ("helix.debug.http.url", bounded_debug_string(&req.url)),
        ("helix.debug.http.headers", bounded_debug_string(&headers)),
    ];
    if let Some(body) = &req.body {
        attributes.push(("helix.debug.http.body_base64", bounded_debug_bytes(body)));
    }
    attributes
}

/// full_debug 下记录本地路径、上传地址与请求头；safe 模式不进入 OTLP。
fn full_debug_upload_attributes(req: &FileUploadRequest) -> Vec<(&'static str, String)> {
    let headers = serde_json::to_string(&req.headers).unwrap_or_else(|_| "[]".to_string());
    vec![
        (
            "helix.debug.upload.local_path",
            bounded_debug_string(&req.local_path),
        ),
        (
            "helix.debug.upload.object_key",
            bounded_debug_string(&req.object_key),
        ),
        (
            "helix.debug.upload.url",
            bounded_debug_string(req.urls.upload_url()),
        ),
        (
            "helix.debug.upload.public_url",
            bounded_debug_string(req.urls.public_url()),
        ),
        ("helix.debug.upload.headers", bounded_debug_string(&headers)),
    ]
}

/// full_debug 下使用 base64 保持二进制帧可回放；返回值始终有界。
fn bounded_debug_bytes(bytes: &[u8]) -> String {
    let shown = bytes.len().min(FULL_DEBUG_PAYLOAD_LIMIT);
    let mut value = base64_encode(&bytes[..shown]);
    if shown < bytes.len() {
        value.push_str(&format!("...[truncated {} bytes]", bytes.len() - shown));
    }
    value
}

struct NoopTraceHooks;

impl TraceHooksImpl for NoopTraceHooks {
    fn on_tick_start(&self, _: &Tick, _: Option<TraceCarrier>) -> TraceScope {
        TraceScope::noop()
    }

    fn on_storage_dispatch(&self, _: Option<Correlation>, _: &[StorageOp]) {}

    fn on_http_dispatch(&self, _: Option<Correlation>, _: &mut HttpRequest) {}

    fn on_upload_dispatch(&self, _: Option<Correlation>, _: &FileUploadRequest) {}

    fn on_ws_send(&self, _: TransportId, _: &mut Bytes) {}

    fn on_event_emit(&self, _: &DomainEventBytes) {}
}

#[cfg(test)]
mod tests {
    use bytes::Bytes;
    use helix_core::tick::AppCommand;
    use helix_core::Tick;

    use super::{bounded_command_name, tick_root_span_name};

    /// Command Tick 的根 Span 名称必须带 Helix 命名空间和受控命令名。
    #[test]
    fn command_tick_root_span_uses_namespaced_command_name() {
        let tick = Tick::Command(AppCommand::new(
            "im:post:sending",
            Bytes::from_static(b"{}"),
        ));

        assert_eq!(tick_root_span_name(&tick), "helix.command.im:post:sending");
    }

    /// command name 进入 Trace 前必须保持可见且有界。
    #[test]
    fn command_name_is_bounded_for_trace_attributes() {
        assert_eq!(bounded_command_name("im:post:sending"), "im:post:sending");
        assert_eq!(bounded_command_name(&"x".repeat(300)).len(), 256);
    }
}
