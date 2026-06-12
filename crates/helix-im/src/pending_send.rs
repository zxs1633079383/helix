//! PendingSend 聚合（发送对账的一致性边界）
//!
//! ## C006 收敛保证
//!
//! - echo 先到 → CancelTimer 让 15s 永不 fire
//! - timer 先到 → 标 UnSend；迟到 echo 仍按 temporaryId 覆盖回 Sent
//! - 两种次序终态一致，纯状态机确定性保证，不依赖时序
//!
//! ## 源码印证（2026-06-08 实测）
//!
//! - ON CONFLICT(temporary_id) DO UPDATE ... —— temporaryId 作 PK
//! - echo/sync 对账走同一对账点（reconcile）
//! - 15s 超时无自动补发（on_timeout 不产出 Http effect）

use crate::state::{SendStatus, ServerId, TemporaryId};
use helix_core::effect::{SqlValue, StorageOp, UpsertSpec};
use helix_core::{Correlation, Effect, EffectSink, TimerId};

/// 待对账的单条消息发送记录
pub struct PendingSend {
    /// 聚合根标识（UI 注入，非 helix-core / helix-im 生成）
    pub temporary_id: TemporaryId,
    pub status: SendStatus,
    /// 关联的 15s 超时令牌（CancelTimer 用）
    pub timeout_timer: TimerId,
    /// 用于写入数据库的 Persist corr（可选，发送时可能尚未落库）
    pub persist_corr: Option<Correlation>,
}

impl PendingSend {
    /// 创建新的待对账记录（Local 状态）
    pub fn new(temporary_id: TemporaryId, timeout_timer: TimerId) -> Self {
        Self {
            temporary_id,
            status: SendStatus::Local,
            timeout_timer,
            persist_corr: None,
        }
    }

    /// echo 或 sync 对账到达（temporaryId 值级覆盖）。
    ///
    /// echo 与 sync 走完全相同 Effect 序列（源码印证）：
    /// 1. Persist{ON CONFLICT(temporary_id) 覆盖 id=serverId, status=sent}
    /// 2. CancelTimer{timeout_timer}（撤 15s 超时）
    pub fn reconcile(&mut self, server_id: ServerId, corr: Correlation, fx: &mut EffectSink) {
        // 吐 Persist（ON CONFLICT temporaryId 覆盖为 server_id + Sent）
        // MAJ-5: table 和 conflict_key 使用 &'static str 常量，避免热路径堆分配
        fx.push(Effect::Persist {
            corr,
            ops: vec![StorageOp::BatchUpsert(UpsertSpec {
                table: "messages",
                rows: vec![vec![
                    (
                        "temporary_id".to_string(),
                        SqlValue::Text(self.temporary_id.0.clone()),
                    ),
                    ("id".to_string(), SqlValue::Integer(server_id.0 as i64)),
                    (
                        "send_status".to_string(),
                        SqlValue::Text("sent".to_string()),
                    ),
                ]],
                // conflict_key 的值（"temporary_id"）由 ACL-1 提供，core 不知道含义
                conflict_key: Some("temporary_id"),
            })],
        });

        // 吐 CancelTimer（撤 15s 超时）
        fx.push(Effect::CancelTimer {
            id: self.timeout_timer,
        });

        self.status = SendStatus::Sent;
    }

    /// 15s 超时触发 → 标 UnSend（无自动补发）。
    ///
    /// C006：若 echo 迟到（status 已为 Sent），幂等忽略。
    pub fn on_timeout(&mut self, fx: &mut EffectSink) {
        // 幂等：已对账（Sent）或已标记 UnSend 的，重复 timer fire 忽略
        if self.status == SendStatus::Sent || self.status == SendStatus::UnSend {
            return;
        }

        self.status = SendStatus::UnSend;

        // PersistFire：batch_update send_status = unsend（幂等）
        fx.push(Effect::PersistFire {
            ops: vec![StorageOp::BatchUpsert(UpsertSpec {
                table: "messages",
                rows: vec![vec![
                    (
                        "temporary_id".to_string(),
                        SqlValue::Text(self.temporary_id.0.clone()),
                    ),
                    (
                        "send_status".to_string(),
                        SqlValue::Text("unsend".to_string()),
                    ),
                ]],
                conflict_key: Some("temporary_id"),
            })],
        });
    }
}
