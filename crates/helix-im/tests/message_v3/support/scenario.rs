//! MessageV3 Gate 回放与 Effect 捕获支持。

use helix_core::effect::{Effect, StorageOp};
use helix_core::tick::PortOutcome;
use helix_core::{Correlation, EffectSink, Module, Tick};
use helix_im::event::MessageV3Event;
use helix_im::{ImConfig, ImModule};
use serde_json::{json, Value};

/// 一次真实 `ImModule::handle` 回放记录，保留 Tick 与完整 Effect 边界。
pub struct Scenario {
    module: ImModule,
    sink: EffectSink,
    ticks: Vec<Value>,
}

impl Scenario {
    /// 创建使用注入身份和确定性时间的 sans-IO 回放器。
    pub fn new(config: ImConfig) -> Self {
        Self {
            module: ImModule::new(config),
            sink: EffectSink::new(),
            ticks: Vec::new(),
        }
    }

    /// 通过生产公共编排排队业务写和 terminal MessageV3 events。
    pub fn queue_commit(
        &mut self,
        ops: Vec<StorageOp>,
        terminal_events: Vec<MessageV3Event>,
        now_ms: u64,
    ) -> Correlation {
        self.sink.clear();
        let corr = self
            .module
            .queue_message_v3_commit(ops, terminal_events, &mut self.sink);
        self.ticks.push(json!({
            "kind": "Command",
            "value": {"name": "__message_v3_commit__", "correlation": corr.raw()},
            "now_ms": now_ms
        }));
        corr
    }

    /// 把驱动持久化/HTTP 结果作为真实 Tick 回灌生产模块。
    pub fn reply(
        &mut self,
        corr: Correlation,
        outcome: PortOutcome,
        now_ms: u64,
    ) -> Result<(), helix_core::CoreError> {
        self.sink.clear();
        let result = self.module.handle(
            &Tick::PortReply {
                corr,
                outcome: outcome.clone(),
            },
            now_ms,
            &mut self.sink,
        );
        self.ticks.push(json!({
            "kind": "PortReply",
            "value": {
                "correlation": corr.raw(),
                "outcome": match outcome {
                    PortOutcome::Ok(reply) => json!({
                        "status": "ok",
                        "readback": String::from_utf8_lossy(reply.0.as_ref())
                    }),
                    PortOutcome::Err(error) => json!({
                        "status": "error",
                        "error": format!("{error:?}")
                    })
                }
            },
            "now_ms": now_ms
        }));
        result
    }

    /// 返回当前 step 的完整 Effect，测试据此验证 Persist/Emit 边界。
    pub fn effects(&self) -> &[Effect] {
        self.sink.as_slice()
    }

    /// 返回确定性入站 Tick JSON，供 Gate artifact 直接落盘。
    pub fn ticks(&self) -> &[Value] {
        &self.ticks
    }

    /// 从当前 step 的生产 Emit 解出 persona 事件，不重建业务 payload。
    pub fn emitted_events(&self) -> Vec<Value> {
        self.sink
            .as_slice()
            .iter()
            .filter_map(|effect| {
                let Effect::Emit { event } = effect else {
                    return None;
                };
                serde_json::from_slice(event.0.as_ref()).ok()
            })
            .collect()
    }
}
