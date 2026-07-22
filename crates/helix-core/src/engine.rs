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
            // EFFECT-3：start() 跨模块**不 clear** scratch（要累积返回所有模块的 on_start
            // Effect），故每个模块只能注册自己**新增**区间 [start..]，否则后一个模块的
            // register 会把前序模块的 Correlation 重复扫描并覆盖到自己 → PortReply 投错家。
            let start = self.scratch.as_slice().len();
            self.host.start_one(idx, &mut self.scratch)?;
            self.register_effects_to_maps(idx, start);
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
                        // start=0：step() 开头已 clear，scratch 内全是本次新增（EFFECT-3）
                        self.register_effects_to_maps(idx, 0);
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
                        self.register_effects_to_maps(module_index, 0);
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
                        // one-shot-on-fire 语义（HIGH 修复，2026-06-22）：timer 触发即作废。
                        // 全仓无 recurring timer——"周期心跳"由模块吐**新 id** 的 ScheduleTimer
                        // re-arm（如 IM ping），新 id 经 register_effects_to_maps 注册新条目，
                        // 绝不复用旧 id。故 fire 后必须回收已触发条目，否则 timer_map 单调泄漏。
                        self.host
                            .dispatch_reply(module_index, &tick, now_ms, &mut self.scratch)?;
                        // 不变量：必须先 remove(已触发 id)、后 register 新 effect。
                        // re-arm 用全新 id（不复用 timer_id）→ 两步互不干扰；
                        // 若极端情况下 re-arm 复用了同一 id（当前全仓无此模式），
                        // 此顺序会误删 —— 故顺序固定，新语义须显式 RecurringTimer 而非复用 id。
                        self.timer_map.remove(timer_id);
                        // Timer 产出的新 Effect（如 re-arm 的新 id ScheduleTimer）注册到同一模块
                        self.register_effects_to_maps(module_index, 0);
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

    /// 扫描 scratch `[start..]` 区间内本次 dispatch 产出的新 Effect，
    /// 把 Correlation/TimerId 注册到 `module_index` 对应的 map。
    ///
    /// ## BLK-1a 修复
    ///
    /// 参数 `module_index` 来自 dispatch 函数的返回值，不再是硬编码的 `0`。
    /// 确保多模块并存时，每个模块的 Correlation/TimerId 路由到自己而非 module[0]。
    ///
    /// ## EFFECT-3 修复
    ///
    /// 参数 `start` = 本次 dispatch 前的 scratch 长度，只扫**新增**区间。
    /// `step()` 开头 clear 故传 `0`；`start()` 跨模块累积 scratch 故传各模块 on_start 前的长度，
    /// 避免后一个模块把前序模块的 Correlation 重复扫描并覆盖到自己（PortReply 投错家）。
    fn register_effects_to_maps(&mut self, module_index: usize, start: usize) {
        for effect in &self.scratch.as_slice()[start..] {
            match effect {
                Effect::Persist { corr, .. }
                | Effect::PersistAtomic { corr, .. }
                | Effect::Http { corr, .. }
                | Effect::UploadFile { corr, .. }
                | Effect::Request { corr, .. } => {
                    self.corr_map.register(*corr, module_index);
                }
                Effect::ScheduleTimer { id, .. } => {
                    self.timer_map.register(*id, module_index);
                }
                Effect::CancelTimer { id } => {
                    self.timer_map.remove(id);
                }
                // HttpFire 故意不注册 corr：它无 Correlation、不产 PortReply（fire-and-forget），
                // 与 Emit / PersistFire 同列落通配臂。
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

#[cfg(test)]
impl ExecutionShell {
    /// 测试探针：当前 timer_map 路由条目数。
    ///
    /// 用于回放测试断言「连续 N 次 Timer 触发后 timer_map 不随 N 线性增长」
    /// （one-shot 收敛到 0 / re-arm 新 id 稳定到 1，绝非 ==N）。
    /// `#[cfg(test)]` 隔离，不污染生产 API（不破坏 public-api 闸门）。
    pub(crate) fn timer_map_len(&self) -> usize {
        self.timer_map.len()
    }
}

// ─── 确定性单元测试（不依赖任何运行时）──────────────────────────────────────

#[cfg(test)]
#[path = "engine_tests.rs"]
mod tests;
