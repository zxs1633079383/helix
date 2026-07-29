//! Channel ingest and contiguous-buffer state machine.

use super::projection::{collect_channel_update_for_post, visible_post_for_storage};
use super::{emit_for_kind, event_to_storage_op, Channel, Gate, MAX_GATE_BUFFER};
use crate::error::ImError;
use crate::state::{ChannelId, Cursor, Seq};
use crate::sync_session::EventEnvelope;
use helix_core::effect::TimerId;
use helix_core::{Correlation, Effect, EffectSink};
use std::collections::BTreeMap;

impl Channel {
    pub fn new(id: ChannelId, initial_cursor: u64) -> Self {
        Self {
            id,
            cursor: Cursor::new(Seq(initial_cursor)),
            gate: None,
            pending_commits: std::collections::HashMap::new(),
            buffer: BTreeMap::new(),
            inflight_sync: None,
            last_sync_from_seq: None,
            terminal_event_seq: None,
        }
    }

    pub(crate) fn is_terminal(&self) -> bool {
        self.terminal_event_seq.is_some()
    }

    pub(crate) fn terminal_event_seq(&self) -> Option<Seq> {
        self.terminal_event_seq
    }

    /// 从启动 scan 恢复 closed tombstone。终态水位必须已经被 cursor 同批提交；否则宁可
    /// 不注册该 marker，也不能让一个超前 tombstone 绕过连续游标不变量。
    pub(crate) fn restore_terminal(&mut self, terminal_seq: Seq) -> bool {
        if terminal_seq.0 == 0 || terminal_seq > self.cursor.value() {
            return false;
        }
        self.terminal_event_seq = Some(terminal_seq);
        self.buffer.clear();
        self.gate = None;
        self.last_sync_from_seq = None;
        true
    }

    /// `PersistAtomic` 成功后的内存提交。持久化回执前绝不调用本方法，因此失败时既不推进
    /// cursor，也不把 channel 变成 closed。sync/notify 是 viewer-filtered，terminal 前可包含
    /// 同批可见事件或不可见序号；调用方已验证整批严格递增并把可见前缀与 tombstone 同事务写入。
    pub(crate) fn commit_terminal_after_atomic(&mut self, terminal_seq: Seq) -> bool {
        if self.is_terminal() || terminal_seq <= self.cursor.value() {
            return false;
        }
        if !self.cursor.try_advance(terminal_seq) {
            return false;
        }
        self.terminal_event_seq = Some(terminal_seq);
        self.buffer.clear();
        self.gate = None;
        self.last_sync_from_seq = None;
        true
    }

    /// Commit a strictly contiguous, already-durable non-terminal event without issuing a second cursor write.
    /// The caller must have included `advance_cursor_op` in the matching `PersistAtomic` batch.
    pub(crate) fn commit_contiguous_after_atomic(
        &mut self,
        target_seq: Seq,
        fx: &mut EffectSink,
    ) -> Result<bool, ImError> {
        if self.is_terminal() || target_seq != Seq(self.cursor.value().0.saturating_add(1)) {
            return Ok(false);
        }
        if !self.cursor.try_advance(target_seq) {
            return Ok(false);
        }
        let mut channel_updates = Vec::new();
        self.flush_contiguous_from_buffer(fx, &mut channel_updates)?;
        for pending in channel_updates {
            fx.push(crate::acl::to_effect::bump_channel_unread(&pending.update));
        }
        Ok(true)
    }

    /// 处理一个入站事件（WS 路径，§4.6）
    ///
    /// - seq == cursor+1 → 立即 apply（PersistFire）+ flush_contiguous_from_buffer
    /// - seq <= cursor   → dup drop（幂等去重）
    /// - seq > cursor+1  → buffer + arm gate（ScheduleTimer 1s）
    pub fn ingest(
        &mut self,
        ev: EventEnvelope,
        fx: &mut EffectSink,
        now_ms: u64,
    ) -> Result<(), ImError> {
        let mut channel_updates = Vec::new();
        self.ingest_collecting_channel_updates(ev, fx, now_ms, &mut channel_updates)?;
        for pending in channel_updates {
            fx.push(crate::acl::to_effect::bump_channel_unread(&pending.update));
        }
        Ok(())
    }

    /// 生产路径使用的 `ingest` 变体：ChannelUpdate 需要 channel 写后读回累计行，
    /// 因此这里只收集待注册的 correlated Persist，不在同 step 内直接 emit delta patch。
    pub fn ingest_collecting_channel_updates(
        &mut self,
        ev: EventEnvelope,
        fx: &mut EffectSink,
        now_ms: u64,
        channel_updates: &mut Vec<crate::channel_update::PendingChannelUpdate>,
    ) -> Result<(), ImError> {
        if self.is_terminal() {
            tracing::info!(
                channel_id = self.id.as_str(),
                event_seq = ev.seq.0,
                terminal_event_seq = ?self.terminal_event_seq.map(|seq| seq.0),
                "dropping late event for terminal channel"
            );
            return Ok(());
        }
        let expected = Seq(self.cursor.value().0 + 1);
        // —— HOP③ 常驻日志（全链 echo 诊断·③ helix 输出/gate 决策）————————————————
        // 关键区分：收到帧后 gate 是 apply-emit / dup-drop / buffer-gap（buffer 即「收到但不吐投影」
        // = DOM 永不更新的根因）。落 run-app.log，让「断在 gate buffer 这跳」一眼可证。
        let decision = match ev.seq.cmp(&expected) {
            std::cmp::Ordering::Equal => "apply-emit",
            std::cmp::Ordering::Less => "dup-drop",
            std::cmp::Ordering::Greater => "buffer-gap(no-emit)",
        };
        tracing::info!(
            hop = "3-gate",
            ch = %self.id.as_str(),
            seq = ev.seq.0,
            cursor = self.cursor.value().0,
            kind = ev.kind.type_num(),
            decision,
            "HOP3 channel gate decision"
        );
        match ev.seq.cmp(&expected) {
            std::cmp::Ordering::Equal => {
                self.apply_and_flush_contiguous(ev, fx, now_ms, channel_updates)
            }
            std::cmp::Ordering::Less => Ok(()), // dup drop，幂等
            std::cmp::Ordering::Greater => self.buffer_and_arm_gate(ev, fx),
        }
    }

    /// 为 MessageV3 post 保留连续性门控，但把持久提交与 Emit 交还 module correlation 编排。
    pub(crate) fn admit_message_v3_post(
        &mut self,
        event: EventEnvelope,
        effects: &mut EffectSink,
    ) -> Result<Option<EventEnvelope>, ImError> {
        if self.is_terminal() {
            return Ok(None);
        }
        let expected = Seq(self.cursor.value().0.saturating_add(1));
        match event.seq.cmp(&expected) {
            std::cmp::Ordering::Equal => Ok(Some(event)),
            std::cmp::Ordering::Less => Ok(None),
            std::cmp::Ordering::Greater => {
                self.buffer_and_arm_gate(event, effects)?;
                Ok(None)
            }
        }
    }

    /// matching PersistOk 后推进 cursor，并交出下一条已连续的缓冲事件。
    pub(crate) fn commit_message_v3_post(&mut self, committed_seq: Seq) -> Option<EventEnvelope> {
        if committed_seq != Seq(self.cursor.value().0.saturating_add(1))
            || !self.cursor.try_advance(committed_seq)
        {
            return None;
        }
        let next_seq = Seq(committed_seq.0.saturating_add(1));
        self.buffer.remove(&next_seq)
    }

    /// 持久化失败时把未提交事件放回 gate，等待补偿同步而不推进 cursor。
    pub(crate) fn restore_message_v3_post(
        &mut self,
        event: EventEnvelope,
        effects: &mut EffectSink,
    ) {
        self.buffer.insert(event.seq, event);
        self.arm_gate_if_needed(effects);
    }

    /// BLOCKING-2：仅推进 cursor 的 gate 入口（posts_update 批量帧用）。
    ///
    /// posts_update 是**单 cursor 事件**（顶层一个 event_seq）但携带 N 条 post 编辑/撤回——
    /// 真源 `handle_batch_update`（post.rs:226）保存所有 post 后由顶层 event_seq 推进 cursor 一次。
    /// N 条内容 patch 由调用方**立即**幂等落库（`BatchUpdate WHERE id`，本地无行=no-op），本方法
    /// 只负责 cursor 的严格 +1 / drop / 缺口 arm-gate——**不**落 message、**不** emit（emit 由调用方）。
    ///
    /// 三路（与 `ingest` 同口径，守 spec §S6 严格 +1）：
    /// - seq == cursor+1 → 推进 cursor（+ flush buffer 中已就绪的连续 post）；
    /// - seq <= cursor   → drop（幂等，cursor 不动）；
    /// - seq >  cursor+1 → 缺口：**仅 arm 1s gate**（不 buffer——内容已立即落库，缺的是中间 post，
    ///   由 post 路径 buffer 或 gate 触发的 sync 回拉填上；cursor 卡住不前进直至缺口填平，
    ///   绝不 MAX-jump 跨过空洞）。
    pub fn ingest_cursor_advance(&mut self, seq: Seq, fx: &mut EffectSink) -> Result<(), ImError> {
        if self.is_terminal() {
            tracing::info!(
                channel_id = self.id.as_str(),
                event_seq = seq.0,
                terminal_event_seq = ?self.terminal_event_seq.map(|terminal| terminal.0),
                "dropping late cursor advance for terminal channel"
            );
            return Ok(());
        }
        let expected = Seq(self.cursor.value().0 + 1);
        match seq.cmp(&expected) {
            std::cmp::Ordering::Equal => {
                self.cursor.try_advance(seq);
                fx.push(crate::acl::to_effect::advance_cursor(self.id, seq));
                let mut channel_updates = Vec::new();
                self.flush_contiguous_from_buffer(fx, &mut channel_updates)?;
                for pending in channel_updates {
                    fx.push(crate::acl::to_effect::bump_channel_unread(&pending.update));
                }
                // 缺口已被本帧填平 → 若无后续缺口，gate 由 flush 后的连续性自然失效；
                // 这里不主动 cancel gate（与 ingest 一致，gate timer 触发时再判定 still-stuck）。
                Ok(())
            }
            std::cmp::Ordering::Less => Ok(()), // dup drop
            std::cmp::Ordering::Greater => {
                // 缺口：不 buffer 内容（已立即落库），仅 arm gate 触发 sync 回拉缺口。
                self.arm_gate_if_needed(fx);
                Ok(())
            }
        }
    }

    /// 处理同步原子写成功回执，并交出下一条刚变连续的实时事件。
    ///
    /// sync 权威路径的 commit 阶段：
    /// 1. 从 pending_commits 取出 target_seq
    /// 2. commit_cursor（MAX guard）
    /// 3. 把下一条实时事件交给 MessageV3 correlated persistence，禁止在此绕回 fire-and-forget。
    pub fn on_persist_ok(&mut self, corr: Correlation) -> Result<Option<EventEnvelope>, ImError> {
        if let Some(target_seq) = self.pending_commits.remove(&corr) {
            self.commit_cursor(target_seq)?;
            let next_seq = Seq(self.cursor.value().0.saturating_add(1));
            return Ok(self.buffer.remove(&next_seq));
        }
        Ok(None)
    }

    // ─── 私有方法 ──────────────────────────────────────────────────────────────

    /// apply_and_flush_contiguous（WS 直连路径）
    ///
    /// WS 路径：事件严格连续，直接 PersistFire（幂等，不需要 PortReply）
    /// 并立即推进内存 cursor，然后尝试 flush buffer 中连续段。
    fn apply_and_flush_contiguous(
        &mut self,
        ev: EventEnvelope,
        fx: &mut EffectSink,
        _now_ms: u64,
        channel_updates: &mut Vec<crate::channel_update::PendingChannelUpdate>,
    ) -> Result<(), ImError> {
        let seq = ev.seq;

        let visible_post = visible_post_for_storage(&ev);
        if visible_post {
            // 吐 PersistFire（幂等落库）。BLOCKING-2 kind-aware：type1=全量 upsert / type2=内容 patch
            // /type3=撤回 / type6=已读位覆盖——edit/read 经 gate 缓冲后 flush 也按各自语义落库，
            // 不再误用全量 upsert 擦本地权威 read_bits/send_status。WS 路径用 PersistFire（幂等）。
            fx.push(Effect::PersistFire {
                ops: vec![event_to_storage_op(&ev)],
            });
        }

        // ⑤：未读 bump（handler 算定，gate 单一发出点）。立即 apply 与 flush 共用此逻辑——
        // 乱序经 buffer 的消息 flush 时同样补未读。`None`（echo / 非新消息 / sync）→ 不发。
        // GuardedBump 守卫（create_at > last_root_post_at）兜底幂等，重复/旧消息不重复 +1。
        collect_channel_update_for_post(&ev, "online_post", channel_updates);

        // 推进内存 cursor（WS 路径：直接 try_advance，不等 PortReply）
        // 理由：WS 顺序可靠，PersistFire 幂等（写 678 message 表，与 sync 路径同表同 PK）；
        // 异常落库失败时由 sync 路径从 cursor 重拉同事件覆盖恢复（两路径表名一致，恢复闭环成立）
        self.cursor.try_advance(seq);

        // PersistFire monotonic_upsert（持久化 cursor，幂等）
        fx.push(crate::acl::to_effect::advance_cursor(self.id, seq));

        if visible_post {
            // Emit 通知前端（kind-aware：post=received / edit=updated / revoke=deleted / read=read）。
            fx.push(emit_for_kind(&ev));
        }

        // 尝试放行 buffer 中紧接着的连续段
        self.flush_contiguous_from_buffer(fx, channel_updates)?;

        Ok(())
    }

    /// buffer_and_arm_gate：将乱序事件放入缓冲，arm 1s gate timer
    ///
    /// ## BLK-1b 约定
    ///
    /// TimerId 由 `channel_id * 1000 + (cursor & 0xFFF)` 派生，确保同一 channel
    /// 在不同 cursor 位置的 gate timer 有不同 id（避免 timer_map 冲突）。
    /// 更健壮的方案是从 module 级别的 IdSource 分配；此处先用 channel 内派生策略。
    fn buffer_and_arm_gate(
        &mut self,
        ev: EventEnvelope,
        fx: &mut EffectSink,
    ) -> Result<(), ImError> {
        // E6：客户端 buffer 容量安全网。缺口过大（远超 gate 正常回填规模）时，
        // 不再无界堆积、不再只等服务端 too_long——主动走 too_long 恢复：
        // 清 buffer + 通知 UI 全量刷新 + 保持 gate（cursor 有效不回退，sync 从
        // cursor+1 重拉缺口）。丢弃的 far-future 事件由后续 sync/WS 重新投递。
        // 退化场景（永久缺口 + 持续洪峰）每满一次触发一次恢复，触发频率有界。
        if self.buffer.len() >= MAX_GATE_BUFFER {
            self.buffer.clear();
            // 客户端 buffer 溢出自触发 too_long：cursor 不回退（保持现值，注释 §E6）。
            // 故 resetTo = cursor+1 → 前端 cursor=resetTo-1 = 现值不变；UI 仍刷新首屏，
            // sync 从 cursor+1 重拉缺口（与服务端 too_long 的 reset_to 语义统一走同一 emit）。
            // 只重置 Dialog 派生字段，不物理删除 message。reload 失败时保留旧历史，
            // 成功后由 getLatestPost 的 BatchUpsert 覆盖最终态。
            let reset_to = Seq(self.cursor.value().0 + 1);
            fx.push(Effect::PersistFire {
                ops: vec![crate::acl::to_effect::reset_channel_dialog_op(self.id)],
            });
            fx.push(crate::acl::to_effect::emit_sync_too_long(self.id, reset_to));
            self.arm_gate_if_needed(fx);
            return Ok(());
        }

        self.buffer.insert(ev.seq, ev);
        self.arm_gate_if_needed(fx);
        Ok(())
    }

    /// arm 1s gate timer（若尚未 armed）。
    ///
    /// BLK-1b：TimerId 由 channel_id 偏移 + cursor 低位派生，避免多 channel timer 碰撞。
    fn arm_gate_if_needed(&mut self, fx: &mut EffectSink) {
        if self.gate.is_none() {
            let timer_id = TimerId::from_raw(
                self.id
                    .hash_u64()
                    .wrapping_mul(1000)
                    .wrapping_add(self.cursor.value().0 & 0xFFF),
            );
            fx.push(Effect::ScheduleTimer {
                id: timer_id,
                after_ms: 1_000,
            });
            self.gate = Some(Gate {
                timer_id,
                expected_seq: Seq(self.cursor.value().0 + 1),
            });
        }
    }

    /// flush_contiguous_from_buffer：从 buffer 中取出严格连续段并 apply
    ///
    /// 每次 cursor 推进后调用：尝试将 buffer 中从 cursor+1 开始的连续段
    /// 一次性全部放行，每个事件走 PersistFire（幂等）。
    fn flush_contiguous_from_buffer(
        &mut self,
        fx: &mut EffectSink,
        channel_updates: &mut Vec<crate::channel_update::PendingChannelUpdate>,
    ) -> Result<(), ImError> {
        loop {
            let next_expected = Seq(self.cursor.value().0 + 1);
            if let Some(ev) = self.buffer.remove(&next_expected) {
                let seq = ev.seq;
                let visible_post = visible_post_for_storage(&ev);
                // BLOCKING-2：flush 同样 kind-aware（缓冲的乱序事件可能是 edit/read/revoke）。
                if visible_post {
                    fx.push(Effect::PersistFire {
                        ops: vec![event_to_storage_op(&ev)],
                    });
                }
                // ⑤：乱序 post 经 buffer flush 时补未读——与立即 apply 同口径（gate 单一发出点，
                // 修复偏差：原 flush 路径只落 message 不补 channel 未读）。`None`→不发。
                collect_channel_update_for_post(&ev, "online_post", channel_updates);
                self.cursor.try_advance(seq);
                fx.push(crate::acl::to_effect::advance_cursor(self.id, seq));
                if visible_post {
                    fx.push(emit_for_kind(&ev));
                }
            } else {
                break;
            }
        }
        Ok(())
    }
}
