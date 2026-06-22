//! # module_host.rs
//!
//! `Module` trait 和 `ModuleHost`——模块注册、路由、生命周期。
//!
//! ## Module trait 设计（审查意见2 P1 + S1 落实）
//!
//! - 统一 `&Tick<'_>` 入参，消除原草稿 `InboundFrame` 中间层
//! - 删除 `tick()` 方法：周期行为通过 `ScheduleTimer` self-arm 表达
//! - `accepts()` 仅用于 `Inbound` / `Command` 路由；`PortReply` / `Timer` 走 corr_map 定向路由
//!
//! ## 路由策略（审查意见2 P3 落实）
//!
//! - `Inbound` / `Command`：遍历 modules，找第一个 `accepts()` 返回 true 的模块
//! - `PortReply`：通过 `corr_map`（Correlation → module_index）O(1) 定向投递
//! - `Timer`：通过 `timer_map`（TimerId → module_index）O(1) 定向投递
//!
//! 预期模块数 < 10，Inbound 的线性扫描实践 O(1)，可接受。

use crate::effect::{Correlation, EffectSink, TimerId};
use crate::error::CoreError;
use crate::platform::MaybeSend;
use crate::tick::Tick;
use std::collections::HashMap;

/// 确定性业务模块的完整契约。
///
/// ## 实现约束（不可违反）
///
/// - `accepts` 和 `handle` 必须是纯函数（无 I/O，无 await，无 spawn）
/// - `handle` 通过 `&mut EffectSink` 输出 Effect，不直接调用任何 port
/// - 周期心跳：在 `on_start` 吐 `ScheduleTimer{id, after_ms}`，
///   在 `handle(Tick::Timer{id})` 重新 arm，实现无限循环心跳
pub trait Module: MaybeSend + 'static {
    /// 模块唯一名称（用于日志 / 调试 / error 报告）
    fn name(&self) -> &'static str;

    /// 路由判定：此模块是否处理该 Tick。
    ///
    /// ## 约束
    ///
    /// - 仅用于 `Tick::Inbound` 和 `Tick::Command` 的路由
    /// - `Tick::PortReply` 和 `Tick::Timer` **不调用此方法**，走 corr_map 定向路由
    /// - 必须是纯函数（不修改 self）
    fn accepts(&self, tick: &Tick) -> bool;

    /// 处理一个 Tick，将 Effect 写入 `out`。
    ///
    /// ## 不变量（不可违反）
    ///
    /// - 严格同步：零 await，零 I/O，零 spawn
    /// - 不直接调用任何 port trait
    /// - 所有副作用通过 `out.push(Effect::...)` 表达
    fn handle(&mut self, tick: &Tick, now_ms: u64, out: &mut EffectSink) -> Result<(), CoreError>;

    /// 模块启动钩子。
    ///
    /// 在 `engine.start()` 时调用，输出初始 Effect：
    /// - `Effect::Send{hello 帧}`（WS 握手）
    /// - `Effect::Persist{get cursor, corr}`（读取持久化状态）
    /// - `Effect::ScheduleTimer{ping, 8s}`（arm 初始心跳）
    fn on_start(&mut self, out: &mut EffectSink) -> Result<(), CoreError> {
        let _ = out;
        Ok(())
    }

    /// 模块停止钩子。
    ///
    /// 输出清理 Effect：
    /// - `Effect::CancelTimer` 所有 timer
    /// - `Effect::Send{close 帧}`
    fn on_stop(&mut self, out: &mut EffectSink) -> Result<(), CoreError> {
        let _ = out;
        Ok(())
    }
}

// ─── CorrelationMap ──────────────────────────────────────────────────────────

/// `Correlation → module_index` 映射，供 PortReply 定向路由。
///
/// 每次 `step()` 产出含 Correlation 的 Effect（Persist/Http），
/// `ExecutionShell` 调用 `register()` 记录映射。
/// PortReply 到达时，`lookup()` O(1) 查找目标 module_index。
#[derive(Default)]
pub(crate) struct CorrelationMap {
    map: HashMap<Correlation, usize>,
}

impl CorrelationMap {
    pub fn register(&mut self, corr: Correlation, module_index: usize) {
        self.map.insert(corr, module_index);
    }

    /// 消费（移除）：PortReply 处理完毕后移除，避免 stale corr 积累。
    /// MIN-8：原名 `lookup` 语义不明，改为 `consume` 体现"取出并移除"。
    pub fn consume(&mut self, corr: &Correlation) -> Option<usize> {
        self.map.remove(corr)
    }
}

/// `TimerId → module_index` 映射，供 Timer 定向路由。
///
/// ## 语义真源：timer 一律 one-shot-on-fire（HIGH 修复，2026-06-22）
///
/// 全仓**无 recurring timer**——所有 timer 触发即作废，"周期心跳"由模块在
/// `handle(Tick::Timer)` 内吐**全新 id** 的 `ScheduleTimer` re-arm 实现（如 IM ping），
/// 新 id 经 `register_effects_to_maps` 注册新条目，**绝不复用旧 id**。
/// 因此 fire 后旧条目永远应回收：`engine` 在 `dispatch_reply` 后统一 `remove(fired_id)`。
/// 不存在"同 id 复用且 fire 后不重新 ScheduleTimer"的模式（如未来要引入，须新增
/// 显式 RecurringTimer 语义，而非偷偷复用 id；回放测试 + drift-review 守护）。
#[derive(Default)]
pub(crate) struct TimerMap {
    map: HashMap<TimerId, usize>,
}

impl TimerMap {
    pub fn register(&mut self, id: TimerId, module_index: usize) {
        self.map.insert(id, module_index);
    }

    /// 只读查路由（不移除）：派发前定位 module_index。
    /// fire 后由 `engine` 在 `dispatch_reply` 之后统一 `remove(fired_id)` 回收
    /// （one-shot-on-fire 语义，见 struct doc），故此处只读，移除职责归 engine。
    pub fn peek(&self, id: &TimerId) -> Option<usize> {
        self.map.get(id).copied()
    }

    /// 移除一条路由条目。两个调用点：
    /// - `engine` 在 Timer 命中、派发完毕后回收已触发的 one-shot id（防永久泄漏）
    /// - `register_effects_to_maps` 处理模块吐出的 `Effect::CancelTimer`
    pub fn remove(&mut self, id: &TimerId) {
        self.map.remove(id);
    }

    /// 测试探针：当前路由条目数。仅用于回放测试断言 timer_map 不随触发次数线性增长，
    /// `#[cfg(test)]` 隔离不污染生产 API（不破坏 public-api 闸门）。
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.map.len()
    }
}

// ─── ModuleHost ──────────────────────────────────────────────────────────────

/// 模块注册与路由中枢。
///
/// ## 开闭原则（轴④）
///
/// 加新业务模块 = 调 `register()`，不改 `dispatch_*` 函数一行。
pub struct ModuleHost {
    modules: Vec<Box<dyn Module>>,
}

impl ModuleHost {
    pub fn new() -> Self {
        Self {
            modules: Vec::new(),
        }
    }

    /// 注册新业务模块（加模块不改 dispatch 逻辑，开闭原则）
    pub fn register(&mut self, m: impl Module + 'static) {
        self.modules.push(Box::new(m));
    }

    /// 返回已注册的模块数量（供 ExecutionShell::start() 按 index 逐个调用）
    pub fn module_count(&self) -> usize {
        self.modules.len()
    }

    /// 路由 Inbound/Command：线性扫描 accepts()
    /// 返回命中的 module_index（用于 corr_map 注册）
    pub fn dispatch_inbound(
        &mut self,
        tick: &Tick,
        now_ms: u64,
        out: &mut EffectSink,
    ) -> Result<Option<usize>, CoreError> {
        for (idx, m) in self.modules.iter_mut().enumerate() {
            if m.accepts(tick) {
                m.handle(tick, now_ms, out)
                    .map_err(|e| CoreError::ModuleError {
                        module: m.name(),
                        source: Box::new(e),
                    })?;
                return Ok(Some(idx));
            }
        }
        Ok(None)
    }

    /// 定向投递 PortReply（通过 corr_map 查找 module_index）
    pub fn dispatch_reply(
        &mut self,
        module_index: usize,
        tick: &Tick,
        now_ms: u64,
        out: &mut EffectSink,
    ) -> Result<(), CoreError> {
        let m = self
            .modules
            .get_mut(module_index)
            .ok_or(CoreError::NoHandler("module_index out of bounds"))?;
        m.handle(tick, now_ms, out)
            .map_err(|e| CoreError::ModuleError {
                module: m.name(),
                source: Box::new(e),
            })
    }

    /// 调用单个模块的 on_start（BLK-1a：供 ExecutionShell 按 index 逐个调用，
    /// 以便每个模块的 Correlation/TimerId 注册到正确的 module_index）
    pub fn start_one(&mut self, idx: usize, out: &mut EffectSink) -> Result<(), CoreError> {
        let m = self.modules.get_mut(idx).ok_or(CoreError::NoHandler(
            "start_one: module_index out of bounds",
        ))?;
        m.on_start(out).map_err(|e| CoreError::ModuleError {
            module: m.name(),
            source: Box::new(e),
        })
    }

    /// 调用所有模块的 on_start（按注册顺序）
    ///
    /// 注意：此方法不区分各模块的 Correlation；若需要精确路由，
    /// 使用 `start_one(idx)` 逐个调用（ExecutionShell::start 已改为逐个调用）。
    pub fn start_all(&mut self, out: &mut EffectSink) -> Result<(), CoreError> {
        for m in &mut self.modules {
            m.on_start(out).map_err(|e| CoreError::ModuleError {
                module: m.name(),
                source: Box::new(e),
            })?;
        }
        Ok(())
    }

    /// 调用所有模块的 on_stop（按注册逆序，last-in first-out）
    pub fn stop_all(&mut self, out: &mut EffectSink) -> Result<(), CoreError> {
        for m in self.modules.iter_mut().rev() {
            m.on_stop(out).map_err(|e| CoreError::ModuleError {
                module: m.name(),
                source: Box::new(e),
            })?;
        }
        Ok(())
    }
}

impl Default for ModuleHost {
    fn default() -> Self {
        Self::new()
    }
}
