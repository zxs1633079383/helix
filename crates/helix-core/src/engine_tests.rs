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

    fn handle(&mut self, tick: &Tick, _now_ms: u64, out: &mut EffectSink) -> Result<(), CoreError> {
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
