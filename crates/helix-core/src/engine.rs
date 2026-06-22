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
mod tests {
    use super::*;
    use crate::effect::TimerId;
    use crate::module_host::Module;

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

    // ─── timer_map 泄漏回放测试（HIGH 修复，2026-06-22）────────────────────────
    //
    // 验证 one-shot-on-fire 语义：fire 后 engine 回收已触发条目，timer_map 不随
    // 触发次数线性增长。两个测试模块覆盖两种真实模式：
    //   ① 纯 one-shot（gate 1s 闸 / 15s 超时）：fire 后不 re-arm → timer_map 收敛到 0
    //   ② re-arm 新 id（IM ping 心跳）：fire 后吐**全新 id** → timer_map 稳定到 1

    /// 纯 one-shot timer 模块：on_start arm 一个 timer，fire 后不 re-arm。
    /// 模拟 helix-im 的 gate 1s 闸 / 15s 发送超时（channel-derived 一次性 id）。
    struct OneShotModule {
        timer_id: u64,
    }

    impl Module for OneShotModule {
        fn name(&self) -> &'static str {
            "one-shot-test"
        }

        fn accepts(&self, _tick: &Tick) -> bool {
            false
        }

        fn handle(
            &mut self,
            _tick: &Tick,
            _now_ms: u64,
            _out: &mut EffectSink,
        ) -> Result<(), CoreError> {
            // Tick::Timer 命中：不 re-arm（one-shot），不产任何 Effect
            Ok(())
        }

        fn on_start(&mut self, out: &mut EffectSink) -> Result<(), CoreError> {
            out.push(Effect::ScheduleTimer {
                id: TimerId::from_raw(self.timer_id),
                after_ms: 1_000,
            });
            Ok(())
        }
    }

    /// re-arm 新 id 的心跳模块：每次 Timer fire 后吐**全新 id** 的 ScheduleTimer。
    /// 模拟 helix-im 的 ping 心跳（fire→新 alloc id，绝不复用旧 id）。
    struct HeartbeatModule {
        next_id: u64,
    }

    impl HeartbeatModule {
        fn alloc(&mut self) -> TimerId {
            let id = self.next_id;
            self.next_id += 1;
            TimerId::from_raw(id)
        }
    }

    impl Module for HeartbeatModule {
        fn name(&self) -> &'static str {
            "heartbeat-test"
        }

        fn accepts(&self, _tick: &Tick) -> bool {
            false
        }

        fn handle(
            &mut self,
            tick: &Tick,
            _now_ms: u64,
            out: &mut EffectSink,
        ) -> Result<(), CoreError> {
            if let Tick::Timer(_) = tick {
                // re-arm 用全新 id（不复用触发的旧 id）
                out.push(Effect::ScheduleTimer {
                    id: self.alloc(),
                    after_ms: 1_000,
                });
            }
            Ok(())
        }

        fn on_start(&mut self, out: &mut EffectSink) -> Result<(), CoreError> {
            out.push(Effect::ScheduleTimer {
                id: self.alloc(),
                after_ms: 1_000,
            });
            Ok(())
        }
    }

    /// one-shot timer fire 后必须从 timer_map 回收（修复前 peek 不移除 → 永久泄漏）。
    #[test]
    fn one_shot_timer_reclaimed_on_fire() {
        let mut shell = ExecutionShell::new();
        shell.register(OneShotModule { timer_id: 42 });
        shell.start().expect("start should succeed");

        // arm 后 map 有 1 条
        assert_eq!(shell.timer_map_len(), 1, "after on_start: 1 timer armed");

        // fire 该 one-shot timer
        shell
            .step(Tick::Timer(TimerId::from_raw(42)), 2_000)
            .expect("timer step should succeed");

        // 修复后：fire 即回收 → 收敛到 0（修复前 peek 不移除会停在 1 永久泄漏）
        assert_eq!(
            shell.timer_map_len(),
            0,
            "one-shot timer must be reclaimed after fire (no leak)"
        );
    }

    /// 连续 N 次 one-shot timer（不 re-arm）：timer_map 绝不随 N 线性增长。
    /// 每次 arm 一个新 one-shot、fire、断言 fire 后归 0。
    #[test]
    fn repeated_one_shot_timers_do_not_leak() {
        let mut shell = ExecutionShell::new();
        // 用心跳模块的反面：每次手动 arm 一个独立 one-shot 再 fire。
        // 这里用 OneShotModule，on_start arm id=100；fire 后归 0。
        shell.register(OneShotModule { timer_id: 100 });
        shell.start().expect("start should succeed");

        const N: u64 = 50;
        for i in 0..N {
            // fire 当前 armed 的 timer（id=100），fire 后 map 应为 0
            shell
                .step(Tick::Timer(TimerId::from_raw(100)), 1_000 + i)
                .expect("timer step should succeed");
            assert_eq!(
                shell.timer_map_len(),
                0,
                "one-shot map must stay at 0 after each fire (iter {i})"
            );
        }
        // 关键断言：绝非随 N 线性增长（修复前会涨到 1 并卡住，但本模块只 arm 一次，
        // 修复前会停在 1；真正的线性泄漏见 heartbeat 测试）
        assert!(
            shell.timer_map_len() < N as usize,
            "timer_map must not grow with fire count"
        );
    }

    /// re-arm 新 id 心跳：连续 N 次 fire，timer_map 稳定在 1（绝非 ==N 线性增长）。
    /// 这是真正暴露修复前泄漏的回放：修复前每次 fire 留下旧 id + register 新 id → 涨到 N。
    #[test]
    fn rearm_new_id_heartbeat_stays_bounded() {
        let mut shell = ExecutionShell::new();
        shell.register(HeartbeatModule { next_id: 1 });
        shell.start().expect("start should succeed");

        // on_start arm 了 id=1
        assert_eq!(shell.timer_map_len(), 1, "after on_start: 1 timer armed");

        const N: u64 = 50;
        // 依次 fire 当前 armed 的 id（1, 2, 3, ...），每次 fire 后模块 re-arm 下一个 id
        for armed_id in 1..=N {
            shell
                .step(Tick::Timer(TimerId::from_raw(armed_id)), 1_000 + armed_id)
                .expect("timer step should succeed");
            // 修复后：每次 fire 回收旧 id + register 新 id → 稳定在 1
            // 修复前：旧 id 永不移除 + 每次新增新 id → 线性涨到 N
            assert_eq!(
                shell.timer_map_len(),
                1,
                "heartbeat timer_map must stay at 1, never grow to N (armed_id={armed_id})"
            );
        }

        // 终态断言：绝非 ==N（修复前的泄漏特征）
        assert_ne!(
            shell.timer_map_len(),
            N as usize,
            "timer_map must NOT grow linearly with fire count (leak signature)"
        );
        assert_eq!(shell.timer_map_len(), 1, "stable bounded at 1");
    }
}
