//! Channel 聚合（消息 + 游标，一致性边界）
//!
//! ## 聚合不变量（全来自源码事实 2026-06-08）
//!
//! 1. cursor 单调不减（MAX guard）
//! 2. cursor 只在 PortReply(corr, Ok) 后推进——写成功才推
//! 3. seq <= cursor 的入站事件一律 drop（幂等去重）
//! 4. flush_contiguous（WS，严格+1，遇空洞停）vs flush_up_to（sync，跳空洞到权威水位）
//!
//! ## gate 状态机 §4.6
//!
//! - seq == cursor+1     → 立即 apply（PersistFire）+ flush_contiguous_from_buffer
//! - seq <= cursor       → dup drop（幂等）
//! - seq > cursor+1      → buffer + arm 1s gate（ScheduleTimer），1s 后触发 sync/notify
//! - gate 已 armed 再来  → 只入 buffer，不重复 arm
//!
//! ## peek/commit 三阶段（sync 权威路径，§4.6）
//!
//! 1. `peek_up_to(target_seq)`     — 只读预览，返回落库 ops（不推 cursor）
//! 2. 外层吐 `Persist{corr, ops}` — driver 落库
//! 3. `on_persist_ok(corr, fx)`    — 仅 PortReply Ok 才 `commit_up_to` + 推 cursor

use crate::error::ImError;
use crate::state::{ChannelId, Cursor, InflightSync, Seq};
use crate::sync_session::EventEnvelope;
use helix_core::effect::{StorageOp, TimerId};
use helix_core::{Correlation, Effect, EffectSink};
use std::collections::BTreeMap;

/// E6：单 channel gate 乱序缓冲的客户端容量上限。
///
/// 取值须 > 合法最大缺口（承压实测 5000 条乱序堆积由 gate 正常回填），
/// 又须 << OOM 阈值。超限不再无界堆积、不再只依赖服务端 too_long——
/// 主动走 too_long 恢复路径（清 buffer + 通知 UI 全量刷新 + sync 重拉）。
const MAX_GATE_BUFFER: usize = 16_384;

/// gate 缺口闸门（1s 超时后发 /sync/notify）
#[derive(Debug)]
pub struct Gate {
    pub timer_id: TimerId,
    /// 触发时记录的缺口起点（= cursor+1 at gate arm time）
    pub expected_seq: Seq,
}

/// Channel 聚合（消息 + 游标，IM 的一致性边界）
///
/// ## 设计裁决（DDD）
///
/// Message 不是独立聚合根，是 Channel 内实体；Cursor 是 Channel 的值对象。
/// 理由：cursor 推进与消息落库是同一一致性边界（写消息成功才推 cursor），
/// 拆成两个聚合会破坏"写成功才推"不变量。
pub struct Channel {
    pub id: ChannelId,
    pub cursor: Cursor,
    pub gate: Option<Gate>,
    /// corr → target_seq（写成功才推，peek/commit 两段式）
    ///
    /// WS 路径直接走 PersistFire（幂等）；sync 路径走 Persist{corr}，
    /// 此 map 记录 corr → commit_target_seq，PortReply Ok 后推进 cursor。
    pub pending_commits: std::collections::HashMap<Correlation, Seq>,
    /// 乱序缓冲（contiguous flush 用）
    pub buffer: BTreeMap<Seq, EventEnvelope>,
    /// B1 gate_inflight：同 channel 同时只允许一个在途 sync Http。
    ///
    /// Some(InflightSync(corr)) 表示有在途 sync；续拉发出时置 Some，PortReply 到达后清 None。
    /// 防止 needs_continuation 并发自踩（连续触发多个相同 channel 的 sync）。
    pub inflight_sync: Option<InflightSync>,
    /// E3 续拉：记录发出续拉 sync 时的 fromSeq（上一批请求的起点）
    ///
    /// 终止保证：若 persist ok 后 cursor == sync_from_seq（cursor 未前进），
    /// 停止续拉 + log warn（意味着服务端返回的所有事件都低于 fromSeq，幂等丢弃）。
    pub last_sync_from_seq: Option<Seq>,
}

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
        }
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
            std::cmp::Ordering::Equal => self.apply_and_flush_contiguous(ev, fx, now_ms),
            std::cmp::Ordering::Less => Ok(()), // dup drop，幂等
            std::cmp::Ordering::Greater => self.buffer_and_arm_gate(ev, fx),
        }
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
        let expected = Seq(self.cursor.value().0 + 1);
        match seq.cmp(&expected) {
            std::cmp::Ordering::Equal => {
                self.cursor.try_advance(seq);
                fx.push(crate::acl::to_effect::advance_cursor(self.id, seq));
                self.flush_contiguous_from_buffer(fx)?;
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

    /// 处理 PortReply{corr=X, Ok}：推进 cursor（写成功才推）
    ///
    /// sync 权威路径的 commit 阶段：
    /// 1. 从 pending_commits 取出 target_seq
    /// 2. commit_cursor（MAX guard）
    /// 3. flush_contiguous_from_buffer（尝试继续处理缓冲中已就绪的事件）
    pub fn on_persist_ok(&mut self, corr: Correlation, fx: &mut EffectSink) -> Result<(), ImError> {
        if let Some(target_seq) = self.pending_commits.remove(&corr) {
            self.commit_cursor(target_seq, fx)?;
            // 尝试放行 buffer 中已就绪的连续段
            self.flush_contiguous_from_buffer(fx)?;
        }
        Ok(())
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
    ) -> Result<(), ImError> {
        let seq = ev.seq;

        // 吐 PersistFire（幂等落库）。BLOCKING-2 kind-aware：type1=全量 upsert / type2=内容 patch
        // /type3=撤回 / type6=已读位覆盖——edit/read 经 gate 缓冲后 flush 也按各自语义落库，
        // 不再误用全量 upsert 擦本地权威 read_bits/send_status。WS 路径用 PersistFire（幂等）。
        fx.push(Effect::PersistFire {
            ops: vec![event_to_storage_op(&ev)],
        });

        // ⑤：未读 bump（handler 算定，gate 单一发出点）。立即 apply 与 flush 共用此逻辑——
        // 乱序经 buffer 的消息 flush 时同样补未读。`None`（echo / 非新消息 / sync）→ 不发。
        // GuardedBump 守卫（create_at > last_root_post_at）兜底幂等，重复/旧消息不重复 +1。
        if let Some(ref upd) = ev.unread_bump {
            fx.push(crate::acl::to_effect::bump_channel_unread(upd));
        }

        // 推进内存 cursor（WS 路径：直接 try_advance，不等 PortReply）
        // 理由：WS 顺序可靠，PersistFire 幂等（写 678 message 表，与 sync 路径同表同 PK）；
        // 异常落库失败时由 sync 路径从 cursor 重拉同事件覆盖恢复（两路径表名一致，恢复闭环成立）
        self.cursor.try_advance(seq);

        // PersistFire monotonic_upsert（持久化 cursor，幂等）
        fx.push(crate::acl::to_effect::advance_cursor(self.id, seq));

        // Emit 通知前端（kind-aware：post=received / edit=updated / revoke=deleted / read=read）。
        fx.push(emit_for_kind(&ev));

        // 尝试放行 buffer 中紧接着的连续段
        self.flush_contiguous_from_buffer(fx)?;

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
            let reset_to = Seq(self.cursor.value().0 + 1);
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
    fn flush_contiguous_from_buffer(&mut self, fx: &mut EffectSink) -> Result<(), ImError> {
        loop {
            let next_expected = Seq(self.cursor.value().0 + 1);
            if let Some(ev) = self.buffer.remove(&next_expected) {
                let seq = ev.seq;
                // BLOCKING-2：flush 同样 kind-aware（缓冲的乱序事件可能是 edit/read/revoke）。
                fx.push(Effect::PersistFire {
                    ops: vec![event_to_storage_op(&ev)],
                });
                // ⑤：乱序 post 经 buffer flush 时补未读——与立即 apply 同口径（gate 单一发出点，
                // 修复偏差：原 flush 路径只落 message 不补 channel 未读）。`None`→不发。
                if let Some(ref upd) = ev.unread_bump {
                    fx.push(crate::acl::to_effect::bump_channel_unread(upd));
                }
                self.cursor.try_advance(seq);
                fx.push(crate::acl::to_effect::advance_cursor(self.id, seq));
                fx.push(emit_for_kind(&ev));
            } else {
                break;
            }
        }
        Ok(())
    }

    /// peek_up_to（sync 权威路径，只读，不推 cursor）
    ///
    /// 返回 ≤ target_seq 的全部事件的落库 ops（来自 buffer），
    /// 不修改 cursor，不弹出 buffer（peek = 只读预览）。
    ///
    /// 外层对这批 ops 吐 Persist{corr, ops}，PortReply Ok 后才调 commit_up_to。
    pub fn peek_up_to(&self, target_seq: Seq) -> Vec<StorageOp> {
        self.buffer
            .range(..=target_seq)
            .map(|(_, ev)| event_to_upsert_op(ev))
            .collect()
    }

    /// commit_cursor：吐 PersistFire{MonotonicUpsert}，推进内存 cursor
    ///
    /// 仅在 PortReply Ok 后调用（写成功才推原则）。
    fn commit_cursor(&mut self, target_seq: Seq, fx: &mut EffectSink) -> Result<(), ImError> {
        if self.cursor.try_advance(target_seq) {
            // 持久化 cursor（单调）
            fx.push(crate::acl::to_effect::advance_cursor(self.id, target_seq));
            // 清除 buffer 中 ≤ target_seq 的已处理事件（sync 已成功 commit）
            self.buffer.retain(|&seq, _| seq > target_seq);
        }
        Ok(())
    }

    /// too_long 处理：清内存 buffer + cursor 复位 + gate 清除
    ///
    /// 调用方（module.rs）负责吐 Emit(im:sync:too_long)。
    /// 本方法只处理内存状态，SQLite 数据不清（upsert 覆盖）。
    pub fn reset_for_too_long(&mut self, reset_to: Seq, fx: &mut EffectSink) {
        self.buffer.clear();
        self.gate = None;
        // cursor 复位到 reset_to-1（下次 sync 从 reset_to 开始）
        let reset_cursor = Seq(reset_to.0.saturating_sub(1));
        self.cursor = Cursor::new(reset_cursor);
        // PersistFire advance_cursor（too_long 场景 cursor 值降低，monotonic_upsert
        // 的 MAX guard 不会实际写入，但显式调用让意图清晰）
        fx.push(crate::acl::to_effect::advance_cursor(self.id, reset_cursor));
    }
}

// ─── 辅助函数 ────────────────────────────────────────────────────────────────

/// 将 EventEnvelope 转为落库 StorageOp（批量 upsert，ON CONFLICT(temporary_id) DO UPDATE）。
///
/// ## 唯一落库映射（sync 路径与 WS 推送路径共用此函数）
///
/// 写向 678 真库 `message` 表（PK=temporary_id），与发送态完全同表同主键语义——
/// 不再有 "messages" 影子表。两条 live 写路径（WS 推送 / B-rest 增量）经此函数收口，
/// 收到的事件真正落到磁盘 message 表。
///
/// ## 字段映射（真源 = cses-client `From<types::Post> for Message`）
///
/// 字段提取已上移 parser（`extract_post_fields`，`Value` 解析时一次完成），
/// 本函数只读 `ev.fields`（owned typed 视图）拼装 Row——**零再解析、零额外字符串分配**
/// （列名 `&'static str`，字段值 owned 所有权转移）。
///
/// - `temporary_id`（PK）：`fields.temporary_id` ?? `fields.id` ?? `evt:{channel}:{seq}` 合成键。
///   三级回退保证 PK 恒非空、且对同一事件**确定性幂等**（re-sync 同批 = 同 PK 覆盖 = O(1)，
///   不产生重复行、不引发 sync 风暴）。
/// - `channel_id`：`fields.channel_id` 缺省时回退 envelope 权威 channel_id（parser 已校验）。
/// - `props`：保留服务端原始 props 对象（业务扩展）；用于已读位 / event_seq 等下游对账。
///
/// ## 复杂度（诚实标注，HX-C005）
///
/// - **SQL 写**：单条 INSERT ... ON CONFLICT(temporary_id) DO UPDATE = O(1)，无 read-modify-write。
/// - **解析**：0（字段已在 parser `extract_post_fields` 一次提取，本函数不再 `from_slice` 重解析）。
/// - **分配**：字段值由 `ev.fields` clone 出（每条 ~7 个 String clone + 8 个列名 `.to_string()`，
///   均为落库必需的 owned Row 构造，非冗余重解析）；PK 合成兜底分支仅在无 id 时分配一次。
///   注：列名仍走 `String`（`Row = Vec<(String, SqlValue)>` 是 core 类型契约，`convert_row`
///   从 rusqlite 读运行时列名同样是 String；改 `&'static str` 需 core 级动核且破坏 driver
///   codec——超本 drift 修复白名单，见 MAJ-5 仅 table/conflict_key 静态化的同因取舍）。
pub fn event_to_upsert_op(ev: &EventEnvelope) -> StorageOp {
    use helix_core::effect::{SqlValue, UpsertSpec};

    let f = &ev.fields;

    // channel_id 缺省回退到 envelope 权威 channel_id（parser 已校验，含正确性）。
    let channel_id = if f.channel_id.is_empty() {
        ev.channel_id.as_str().to_string()
    } else {
        f.channel_id.clone()
    };

    // PK 三级回退（真源 cses-client：temporary_id ?? id；再加 evt:{ch}:{seq} 合成兜底，
    // 保证收到的事件即便无 temporary_id 也有确定性非空 PK，re-sync 幂等覆盖同行）。
    let pk = if !f.temporary_id.is_empty() {
        f.temporary_id.clone()
    } else if !f.id.is_empty() {
        f.id.clone()
    } else {
        format!("evt:{}:{}", ev.channel_id.as_str(), ev.seq.0)
    };

    StorageOp::BatchUpsert(UpsertSpec {
        table: "message",
        rows: vec![vec![
            ("temporary_id".to_string(), SqlValue::Text(pk)),
            ("id".to_string(), SqlValue::Text(f.id.clone())),
            ("channel_id".to_string(), SqlValue::Text(channel_id)),
            ("user_id".to_string(), SqlValue::Text(f.user_id.clone())),
            ("type".to_string(), SqlValue::Text(f.msg_type.clone())),
            ("message".to_string(), SqlValue::Text(f.message.clone())),
            ("props".to_string(), SqlValue::Text(f.props.clone())),
            ("read_bits".to_string(), SqlValue::Text(f.read_bits.clone())),
            ("create_at".to_string(), SqlValue::Integer(f.create_at)),
        ]],
        conflict_key: Some("temporary_id"),
        exclude_from_update: Vec::new(),
    })
}

/// type=2 编辑：**仅覆盖内容列**，保留本地权威列（C2 真源 message_repo.rs:185-221
/// `update_content_by_message_id`）。
///
/// 真源 cses edit 走 `UPDATE message SET <内容列> WHERE id = msg_id`，**显式不含**
/// `read_bits` / `send_status`（本地权威列）+ 身份列（id/channel_id/user_id/
/// create_at/temporary_id）——对齐 Telegram `applyEdit`「只改内容、不碰已读态」。
///
/// 为何**不能**复用 `event_to_upsert_op`：后者全量 upsert（含 `read_bits=excluded.read_bits`），
/// edit body 的 readBits 常为空（map body 无已读位 → unwrap_or_default → ""），upsert 的
/// ON CONFLICT DO UPDATE 会把已落地行的 read_bits **擦成空串** → 丢已读位（与真源 + 本仓 spec
/// 双重不一致）。改用 `BatchUpdate`（WHERE id IN）做**字段级 patch**，read_bits 列不在 patch
/// 中 → 已读位保留。本地无此行 → no-op（真源 0 行命中=未覆盖，留段查询/远端补拉）。
///
/// patch 范围 = helix `message` 表中 PostFields 携带的内容列：`type` / `message` / `props`
/// （schema.rs 其余内容列如 viewers/mentions/... PostFields 当前不解析 → 不进 patch，避免把
/// 未解析的空值覆盖回库）。复杂度 O(1)（UPDATE WHERE id，idx_message_id 命中）。
pub fn edit_content_op(msg_id: &str, fields: &crate::sync_session::PostFields) -> StorageOp {
    use helix_core::effect::{BatchUpdateSpec, SqlValue};
    StorageOp::BatchUpdate(BatchUpdateSpec {
        table: "message",
        key_col: "id",
        key_vals: vec![SqlValue::Text(msg_id.to_string())],
        // 仅内容列；显式排除 read_bits / send_status（本地权威）+ 身份列。
        patch: vec![
            ("type".to_string(), SqlValue::Text(fields.msg_type.clone())),
            (
                "message".to_string(),
                SqlValue::Text(fields.message.clone()),
            ),
            ("props".to_string(), SqlValue::Text(fields.props.clone())),
        ],
    })
}

/// type=3 撤回：标记 message 行 `revoke=1`（C2 真源 post.rs:1110-1114 `batch_revoke`）。
///
/// 撤回**无须** messages body——直接按 wire msgId 命中行置 revoke。用 `BatchUpdate`
/// （`UPDATE message SET revoke=1 WHERE id IN (msg_id)`）：本地无此行 → no-op（真源
/// `mark_revoked` 返回 Ok(false)，幂等不报错）。**不**用 ON CONFLICT(id)——message 表
/// `id` 仅有非唯一索引 idx_message_id（无 UNIQUE 约束），upsert 冲突子句会运行时报错。
/// 复杂度 O(1)（UPDATE WHERE id，idx_message_id 命中）。
pub fn revoke_op(msg_id: &str) -> StorageOp {
    use helix_core::effect::{BatchUpdateSpec, SqlValue};
    StorageOp::BatchUpdate(BatchUpdateSpec {
        table: "message",
        key_col: "id",
        key_vals: vec![SqlValue::Text(msg_id.to_string())],
        patch: vec![("revoke".to_string(), SqlValue::Integer(1))],
    })
}

/// type=6 已读：**单调覆盖** message 行 `read_bits` 列（C2 真源 post.rs:1116-1120 `batch_apply_read`）。
///
/// read_bits 是服务端权威已读位（从 `messages[msg_id].read_bits` 取），直接赋值**非累加**
/// （HX-C008：read_bits 单调覆盖非累加）。用 `BatchUpdate`（`UPDATE message SET read_bits=?
/// WHERE id IN (msg_id)`）：本地无此行 → no-op（消息体尚未落地，待后续 type1 sync 带其
/// 自身 read_bits 落库）。同样**不**用 ON CONFLICT(id)（message.id 无 UNIQUE 约束）。复杂度 O(1)。
pub fn apply_read_op(msg_id: &str, read_bits: &str) -> StorageOp {
    use helix_core::effect::{BatchUpdateSpec, SqlValue};
    StorageOp::BatchUpdate(BatchUpdateSpec {
        table: "message",
        key_col: "id",
        key_vals: vec![SqlValue::Text(msg_id.to_string())],
        patch: vec![(
            "read_bits".to_string(),
            SqlValue::Text(read_bits.to_string()),
        )],
    })
}

/// post_pin 置顶态：按 message id 幂等 patch props.pinned。
///
/// 本地 message 表无独立 pinned 列，且 post_pin 帧只携 `{postId, channelId, operation}`，没有完整
/// post body；因此只写最小 render-ready 元数据，不复用 `edit_content_op`，避免把正文/type 覆盖为空。
pub fn pin_state_op(msg_id: &str, pinned: bool) -> StorageOp {
    use helix_core::effect::{BatchUpdateSpec, SqlValue};
    let props = serde_json::json!({ "pinned": pinned }).to_string();
    StorageOp::BatchUpdate(BatchUpdateSpec {
        table: "message",
        key_col: "id",
        key_vals: vec![SqlValue::Text(msg_id.to_string())],
        patch: vec![("props".to_string(), SqlValue::Text(props))],
    })
}

/// kind-aware 落库映射：按 `ev.kind` 分派到对应 StorageOp（gate apply/flush 用，BLOCKING-2）。
///
/// 现网真源 `apply_event_with_messages`（post.rs:1146）按 event_type 分发：type1=全量 upsert /
/// type2=内容 patch / type3=撤回 / type6=已读位覆盖。gate buffer 可缓冲任意 kind 的乱序事件
/// （post + edit/read/revoke 都经 `ch.ingest`），flush 必须按各自语义落库——**不能**对 edit/read
/// 也走全量 upsert（会擦本地权威 read_bits/send_status，正是 `edit_content_op` 注释所述擦位 bug）。
///
/// msg_id 取 `ev.msg_id`（wire 权威）?? `ev.fields.id`；edit/read/revoke 缺 msg_id → patch
/// WHERE id="" 命中 0 行 = 安全 no-op（边界零信任）。PostUpsert 用 PK 三级回退恒非空。
pub fn event_to_storage_op(ev: &EventEnvelope) -> StorageOp {
    use crate::sync_session::EventKind;
    let msg_id = ev
        .msg_id
        .as_deref()
        .filter(|s| !s.is_empty())
        .unwrap_or(ev.fields.id.as_str());
    match ev.kind {
        EventKind::PostUpsert => event_to_upsert_op(ev),
        EventKind::PostEdit => edit_content_op(msg_id, &ev.fields),
        EventKind::PostRevoke => revoke_op(msg_id),
        EventKind::PostRead => apply_read_op(msg_id, ev.fields.read_bits.as_str()),
        // 未知类型：cursor 推进但不落 message（phantom 同口径，HX-C008）。空 patch no-op。
        EventKind::Other(_) => StorageOp::BatchUpdate(helix_core::effect::BatchUpdateSpec {
            table: "message",
            key_col: "id",
            key_vals: vec![helix_core::effect::SqlValue::Text(msg_id.to_string())],
            patch: Vec::new(),
        }),
    }
}

/// kind-aware 前端通知 Effect（gate apply/flush 用，BLOCKING-2）。
///
/// 真源 apply_event_with_messages 按 type emit 不同 im:post:* 事件。
/// post=received / edit=updated / revoke=deleted / read=read；未知类型按 received 兜底。
///
/// **spec06 FixB**：传 `&ev.fields`（**在手数据**，与本事件正落库的 `event_to_storage_op(ev)` 同一份
/// owned `PostFields`）给 fat emit 工厂 → 携完整可渲染 MessageItemData，前端零回读直渲。零额外读、零再解析。
pub fn emit_for_kind(ev: &EventEnvelope) -> Effect {
    use crate::sync_session::EventKind;
    let msg_id = ev.msg_id.as_deref().unwrap_or("");
    let seq = ev.seq.0;
    let f = &ev.fields;
    match ev.kind {
        EventKind::PostUpsert => crate::acl::to_effect::emit_post_received(ev.id, seq, msg_id, f),
        EventKind::PostEdit => crate::acl::to_effect::emit_post_updated(ev.id, seq, msg_id, f),
        EventKind::PostRevoke => crate::acl::to_effect::emit_post_deleted(ev.id, seq, msg_id, f),
        EventKind::PostRead => crate::acl::to_effect::emit_post_read(ev.id, seq, msg_id, f),
        EventKind::Other(_) => crate::acl::to_effect::emit_post_received(ev.id, seq, msg_id, f),
    }
}

// ─── 单元测试 ────────────────────────────────────────────────────────────────

#[cfg(test)]
#[path = "gate_tests.rs"]
mod tests;
