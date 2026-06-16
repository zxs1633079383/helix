//! # engine.rs
//!
//! `ExecutionShell`——sans-IO 确定性执行壳（聚合根）。
//!
//! ## 核心不变量
//!
//! - `step()` 严格同步：零 await，零 I/O，零 spawn
//! - `step()` 纯函数语义：相同 Tick 序列 + 相同模块初态 ⇒ 相同 Effect 序列
//! - 维护 `corr_map` 和 `timer_map`，保证 PortReply/Timer 定向投递（审查意见2 P3）

use crate::effect::{Effect, EffectSink};
use crate::error::CoreError;
use crate::module_host::{CorrelationMap, ModuleHost, TimerMap};
use crate::tick::Tick;

/// sans-IO 确定性执行壳。
///
/// ## 使用方式（三端通用）
///
/// ```rust,ignore
/// let mut shell = ExecutionShell::new();
/// shell.register(ImModule::new(config));
/// shell.start()?;
///
/// // 事件泵（由 driver 的 engine_loop.rs 驱动）
/// let effects = shell.step(Tick::Inbound(bytes), now_ms)?;
/// // driver 兑现 effects，把结果包成下一个 Tick 喂回
/// ```
pub struct ExecutionShell {
    host: ModuleHost,
    /// Correlation → module_index（PortReply 定向路由）
    corr_map: CorrelationMap,
    /// TimerId → module_index（Timer 定向路由）
    timer_map: TimerMap,
    /// 跨 step 复用的 Effect 缓冲（零分配，轴①）
    scratch: EffectSink,
}

impl ExecutionShell {
    pub fn new() -> Self {
        Self {
            host: ModuleHost::new(),
            corr_map: CorrelationMap::default(),
            timer_map: TimerMap::default(),
            scratch: EffectSink::new(),
        }
    }

    /// 注册业务模块（加模块不改此函数，开闭原则）
    pub fn register(&mut self, m: impl crate::module_host::Module + 'static) {
        self.host.register(m);
    }

    /// 启动所有已注册模块（调用 on_start），返回初始 Effect 列表。
    ///
    /// driver 在创建完 ExecutionShell 并注册所有模块后调用一次。
    ///
    /// ## BLK-1a 修复
    ///
    /// 逐模块调用 on_start 并立即 `register_effects_to_maps(idx)`，
    /// 确保每个模块的 Correlation/TimerId 注册到正确的 module_index。
    pub fn start(&mut self) -> Result<&[Effect], CoreError> {
        self.scratch.clear();
        let module_count = self.host.module_count();
        for idx in 0..module_count {
            self.host.start_one(idx, &mut self.scratch)?;
            self.register_effects_to_maps(idx);
        }
        Ok(self.scratch.as_slice())
    }

    /// 停止所有已注册模块（调用 on_stop），返回清理 Effect 列表。
    pub fn stop(&mut self) -> Result<&[Effect], CoreError> {
        self.scratch.clear();
        self.host.stop_all(&mut self.scratch)?;
        Ok(self.scratch.as_slice())
    }

    /// **核心方法**：处理一个 Tick，返回本次 step 产出的 Effect 切片。
    ///
    /// ## 返回值生命周期
    ///
    /// 返回 `&[Effect]` 借用 `self.scratch`，在下一次 `step()` 调用前有效。
    /// driver 必须在下一次 `step()` 之前消费完（或 clone）这些 Effect。
    ///
    /// ## 路由策略
    ///
    /// - `Inbound` / `Command`：线性扫描 `accepts()`
    /// - `PortReply`：corr_map O(1) 定向投递
    /// - `Timer`：timer_map O(1) 定向投递
    ///
    /// ## BLK-1a 修复
    ///
    /// `dispatch_inbound` 返回的 `Some(idx)` 现在被正确传入 `register_effects_to_maps`，
    /// 替换了原来的 `let module_index = 0usize; // placeholder` 占位符。
    /// 这确保多模块场景中，Correlation/TimerId 被注册到正确的 module_index，
    /// 而不是错误地路由到 module[0]。
    pub fn step(&mut self, tick: Tick, now_ms: u64) -> Result<&[Effect], CoreError> {
        self.scratch.clear();

        match &tick {
            Tick::Inbound(_) | Tick::Command(_) | Tick::Connected(_) | Tick::Disconnected(_) => {
                match self
                    .host
                    .dispatch_inbound(&tick, now_ms, &mut self.scratch)?
                {
                    Some(idx) => {
                        // BLK-1a: 传入真实的 module_index，而非 0 占位符
                        self.register_effects_to_maps(idx);
                    }
                    None => {
                        // 无模块处理此 Inbound/Command——可能是正常（如未知帧 drop），
                        // 也可能是配置错误。此处不报错，由调用方决定日志级别。
                        tracing::debug!("no module accepted tick");
                    }
                }
                // on_start 也可能产出 Effect，返回前确保 scratch 已被注册
                // (on_start 由 start() 单独调用，此处仅处理 step 路径)
                return Ok(self.scratch.as_slice());
            }

            Tick::PortReply { corr, .. } => {
                match self.corr_map.consume(corr) {
                    Some(module_index) => {
                        self.host
                            .dispatch_reply(module_index, &tick, now_ms, &mut self.scratch)?;
                        // BLK-1a: PortReply 产出的新 Effect 注册到发出原 corr 的同一模块
                        self.register_effects_to_maps(module_index);
                    }
                    None => {
                        // corr 未知：可能是 PersistFire 后的误发（不应发生），
                        // 或模块已停止后的迟到响应（可接受，静默忽略）
                        tracing::warn!(
                            corr = corr.raw(),
                            "received PortReply for unknown correlation, ignoring"
                        );
                    }
                }
                return Ok(self.scratch.as_slice());
            }

            Tick::Timer(timer_id) => {
                match self.timer_map.peek(timer_id) {
                    Some(module_index) => {
                        // recurring timer：不从 timer_map 移除（模块可重新 arm）
                        self.host
                            .dispatch_reply(module_index, &tick, now_ms, &mut self.scratch)?;
                        // BLK-1a: Timer 产出的新 Effect（如 re-arm）注册到同一模块
                        self.register_effects_to_maps(module_index);
                    }
                    None => {
                        // timer_id 未知：已被 CancelTimer 移除后的迟到触发，静默忽略
                        tracing::debug!(
                            id = timer_id.raw(),
                            "received Timer for unknown id, ignoring"
                        );
                    }
                }
                return Ok(self.scratch.as_slice());
            }
        }
    }

    /// 扫描 scratch 中本次 dispatch 产出的新 Effect，
    /// 把 Correlation/TimerId 注册到 `module_index` 对应的 map。
    ///
    /// ## BLK-1a 修复
    ///
    /// 参数 `module_index` 来自 dispatch 函数的返回值，不再是硬编码的 `0`。
    /// 确保多模块并存时，每个模块的 Correlation/TimerId 路由到自己而非 module[0]。
    fn register_effects_to_maps(&mut self, module_index: usize) {
        for effect in self.scratch.as_slice() {
            match effect {
                Effect::Persist { corr, .. }
                | Effect::Http { corr, .. }
                | Effect::Request { corr, .. } => {
                    self.corr_map.register(*corr, module_index);
                }
                Effect::ScheduleTimer { id, .. } => {
                    self.timer_map.register(*id, module_index);
                }
                Effect::CancelTimer { id } => {
                    self.timer_map.remove(id);
                }
                _ => {}
            }
        }
    }
}

impl Default for ExecutionShell {
    fn default() -> Self {
        Self::new()
    }
}

// ─── 确定性单元测试（不依赖任何运行时）──────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// 空 shell，step 任何 Inbound 都返回空 Effect（no-op）
    #[test]
    fn empty_shell_step_no_effects() {
        let mut shell = ExecutionShell::new();
        let effects = shell
            .step(
                Tick::Inbound(crate::tick::InboundBytes::from_static(b"hello")),
                1_000_000,
            )
            .expect("step should not fail on empty shell");
        assert!(effects.is_empty(), "empty shell should produce no effects");
    }
}
