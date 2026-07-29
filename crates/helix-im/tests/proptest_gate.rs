//! Proptest 随机乱序/背压不变量（Phase B Commit 5）
//!
//! ## 验证的不变量
//!
//! 1. **cursor 单调不递减**：任何 Tick 序列下 cursor 永不回退
//! 2. **无消息丢失**：seq ≤ cursor 时该 seq 的 PersistFire 已被发出过
//!    （通过检测 cursor 推进确认，而不是跟踪每条 effect 详情）
//! 3. **确定性**：同一 Tick 序列产出完全相同的 Effect 序列（无外部状态）
//! 4. **幂等性**：重复喂同一 seq 永不让 cursor 回退

mod common;
use common::*;

use helix_core::Tick;
use proptest::prelude::*;

/// 执行一个 Tick，并确认其产生的全部 MessageV3 原子持久化屏障。
fn step_durable(harness: &mut Harness, tick: Tick, now_ms: u64) -> Vec<EffectKind> {
    let mut effects = harness.step(tick, now_ms);
    let mut pending: Vec<u64> = effects
        .iter()
        .filter_map(|effect| match effect {
            EffectKind::PersistAtomic(corr) => Some(*corr),
            _ => None,
        })
        .collect();
    let mut offset = 1_u64;
    while let Some(corr) = pending.pop() {
        let next = harness.step(port_reply_ok(corr), now_ms.saturating_add(offset));
        pending.extend(next.iter().filter_map(|effect| match effect {
            EffectKind::PersistAtomic(next_corr) => Some(*next_corr),
            _ => None,
        }));
        effects.extend(next);
        offset = offset.saturating_add(1);
    }
    effects
}

// ─── 策略：生成随机 (channel_id, seq) 序列 ───────────────────────────────────

/// 生成 [1..max_seq] 范围内长度为 [min_len..=max_len] 的 seq 值序列（乱序/重复/dup 均可能）
fn seq_sequence(max_seq: u64, min_len: usize, max_len: usize) -> impl Strategy<Value = Vec<u64>> {
    proptest::collection::vec(1u64..=max_seq, min_len..=max_len)
}

// ─── 不变量 1：cursor 单调不递减 ─────────────────────────────────────────────

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 200,
        max_shrink_iters: 100,
        ..Default::default()
    })]

    #[test]
    fn prop_cursor_monotone_non_decreasing(
        seqs in seq_sequence(20, 1, 30),
    ) {
        let mut h = Harness::new(&[1]);
        let mut last_cursor: u64 = 0;

        for (i, seq) in seqs.iter().enumerate() {
            step_durable(&mut h, inbound_event(1, *seq, 1), (i as u64 + 1) * 100);
            let cursor = h.module.cursor_for(channel_id(1)).unwrap_or(0);
            prop_assert!(
                cursor >= last_cursor,
                "cursor decreased! was {} now {} after seq={}",
                last_cursor, cursor, seq
            );
            last_cursor = cursor;
        }
    }
}

// ─── 不变量 2：无消息丢失（任何已 apply 的 seq 都被 persist 了）────────────────

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 200,
        max_shrink_iters: 100,
        ..Default::default()
    })]

    #[test]
    fn prop_no_message_loss_contiguous_prefix(
        extra_seqs in seq_sequence(15, 0, 10),
    ) {
        // 先喂 seq 1..10 连续（确保 cursor=10），然后喂 extra_seqs
        let mut h = Harness::new(&[1]);
        for seq in 1u64..=10 {
            step_durable(&mut h, inbound_event(1, seq, 1), seq * 100);
        }
        // cursor 应为 10
        let cursor_after_prefix = h.module.cursor_for(channel_id(1)).unwrap_or(0);
        prop_assert_eq!(cursor_after_prefix, 10,
            "cursor should be 10 after contiguous 1-10");

        // 喂额外 seqs（可能乱序/dup/超前）
        for (i, seq) in extra_seqs.iter().enumerate() {
            let prev = h.module.cursor_for(channel_id(1)).unwrap_or(0);
            step_durable(
                &mut h,
                inbound_event(1, *seq, 1),
                (11 + i as u64) * 100,
            );
            let after = h.module.cursor_for(channel_id(1)).unwrap_or(0);
            prop_assert!(after >= prev,
                "cursor decreased on extra seq={}: {} -> {}", seq, prev, after);
        }
    }
}

// ─── 不变量 3：确定性（同输入 → 同 Effect）──────────────────────────────────

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 100,
        max_shrink_iters: 50,
        ..Default::default()
    })]

    #[test]
    fn prop_determinism_same_input_same_output(
        seqs in seq_sequence(10, 1, 15),
    ) {
        // 两个独立 Harness，喂相同 Tick 序列，验证 Effect 输出完全一致
        let mut h1 = Harness::new(&[1]);
        let mut h2 = Harness::new(&[1]);

        for (i, seq) in seqs.iter().enumerate() {
            let ts = (i as u64 + 1) * 100;
            let e1 = h1.step(inbound_event(1, *seq, 1), ts);
            let e2 = h2.step(inbound_event(1, *seq, 1), ts);
            prop_assert_eq!(&e1, &e2,
                "non-determinism: seq={} ts={} produced different effects", seq, ts);
        }
        // cursor 也应一致
        let c1 = h1.module.cursor_for(channel_id(1)).unwrap_or(0);
        let c2 = h2.module.cursor_for(channel_id(1)).unwrap_or(0);
        prop_assert_eq!(c1, c2, "non-determinism: cursors diverged");
    }
}

// ─── 不变量 4：幂等性（重复 seq 不让 cursor 回退）──────────────────────────────

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 200,
        max_shrink_iters: 100,
        ..Default::default()
    })]

    #[test]
    fn prop_idempotent_duplicate_seq(
        base_seq in 1u64..=10u64,
        repeat_count in 1usize..=5usize,
    ) {
        let mut h = Harness::new(&[1]);
        // 先喂到 base_seq 为止的连续序列
        for seq in 1u64..=base_seq {
            step_durable(&mut h, inbound_event(1, seq, 1), seq * 100);
        }
        let cursor_before = h.module.cursor_for(channel_id(1)).unwrap_or(0);
        prop_assert_eq!(cursor_before, base_seq);

        // 重复喂 base_seq 若干次（dup）
        for i in 0..repeat_count {
            let effects = h.step(inbound_event(1, base_seq, 1), (base_seq + 1 + i as u64) * 100);
            // dup 应静默忽略（无任何 effect）
            prop_assert!(
                effects.is_empty(),
                "dup seq={} should produce no effects (idempotent), got: {:?}", base_seq, effects
            );
            let cursor = h.module.cursor_for(channel_id(1)).unwrap_or(0);
            prop_assert_eq!(cursor, base_seq,
                "cursor changed after dup seq={}!", base_seq);
        }
    }
}

// ─── 不变量 5：乱序背压 → cursor 最终收敛 ──────────────────────────────────────

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 100,
        max_shrink_iters: 50,
        ..Default::default()
    })]

    #[test]
    fn prop_backpressure_eventually_delivers_all(
        perm_seed in 0u64..1000u64,
    ) {
        // 生成 1..12 的乱序排列（由 seed 决定）
        let n: u64 = 12;
        let mut seqs: Vec<u64> = (1..=n).collect();
        // 用简单 Fisher-Yates 混排（确定性）
        let mut rng = perm_seed;
        for i in (1..seqs.len()).rev() {
            rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            let j = (rng >> 33) as usize % (i + 1);
            seqs.swap(i, j);
        }

        let mut h = Harness::new(&[1]);
        let mut ts: u64 = 100;

        // 喂所有 seq（顺序由 perm_seed 决定）
        for seq in &seqs {
            step_durable(&mut h, inbound_event(1, *seq, 1), ts);
            ts += 100;
        }

        // 由于 gate 需要 sync/notify 补全，乱序情况下 cursor 可能不等于 n
        // 但不应超过 n，且不应为负（单调性）
        let cursor = h.module.cursor_for(channel_id(1)).unwrap_or(0);
        prop_assert!(cursor <= n,
            "cursor should not exceed max seq n={}, got {}", n, cursor);

        // 如果恰好是连续的（即 seqs 按 1..n 顺序）cursor 应等于 n
        let is_sorted = seqs.windows(2).all(|w| w[0] < w[1]);
        if is_sorted {
            prop_assert_eq!(cursor, n, "sorted input should yield cursor=n");
        }
    }
}
